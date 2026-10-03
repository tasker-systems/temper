#![cfg(feature = "test-db")]
//! Witnesses for the block history scrub's operator doors (resource erasure build order 2e, D11):
//!
//! * the execute door (`POST /api/admin/resources/block-history-scrub`) — an operator completes,
//!   and the recorded targets equal the survey's prediction; an erased resource and a charter are
//!   recorded refusals whose payload names the scrub (`act`) and its blocks; an empty list, a
//!   repeated block and a block of another resource are 400s that record nothing and empty
//!   nothing, on an erased or charter resource too (membership is checked before any refusal); an unknown field is axum's 422; a scrub of an in-flight segmented ingest answers
//!   `cancelled_ingest: true`, read from the recorded payload.
//! * the survey door (`POST /api/admin/resources/block-history-scrub/survey`) — per-block counts,
//!   nothing recorded; the refusal the act would record for an erased or charter resource; the
//!   same 400s as the act.
//! * a non-operator gets the gate's identical 404 on both doors for an existing, an unknown and
//!   an erased id, and the ledger gains nothing.
//!
//! The setup is `admin_resource_erasure_surface_test.rs`'s; the resource's blocks come from the
//! real create + content-PATCH path (`block_read_handler_test.rs`'s geometry: a filler re-PATCH
//! folds the rewritten-away section, so the resource carries a folded block with history), and
//! the block ids are read from `kb_content_blocks`.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{
    pack_chunks, IngestPayload, PackedChunk, SegmentedBegin, SegmentedBeginResponse,
};

/// Resolve the profile a test JWT's `sub` provisioned.
async fn profile_of_sub(pool: &PgPool, sub: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
    )
    .bind(sub)
    .fetch_one(pool)
    .await
    .expect("the provisioned profile")
}

/// Provision `sub` through a real authenticated request, then return its token and profile.
async fn provision(app: &common::TestApp, sub: &str, email: &str) -> (String, Uuid) {
    let token = common::generate_test_jwt(sub, email);
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("provisioning request");
    assert_eq!(resp.status().as_u16(), 200, "first sign-in must provision");
    let profile = profile_of_sub(&app.pool, sub).await;
    (token, profile)
}

/// An operator: standing plus the governance row that IS `is_system_admin`.
async fn provision_operator(app: &common::TestApp, sub: &str, email: &str) -> (String, Uuid) {
    let (token, profile) = provision(app, sub, email).await;
    common::fixtures::make_test_admin(&app.pool, profile).await;
    (token, profile)
}

/// A non-operator: standing ONLY, so it reaches the gated router but fails the door's gate.
async fn provision_non_operator(app: &common::TestApp, sub: &str, email: &str) -> (String, Uuid) {
    let (token, profile) = provision(app, sub, email).await;
    common::fixtures::approve_standing(&app.pool, profile).await;
    (token, profile)
}

const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";

/// The filler-append rewrite of section A: A's bytes differ so its incumbent folds, while the
/// beta section survives byte-identical and is kept.
fn body_a_with_filler() -> String {
    let filler = "Brand new prose. ".repeat(120);
    format!("# Alpha\n\nAlpha body paragraph.\n\n{filler}\n## Beta\n\nBeta body paragraph.\n")
}

/// Give the resource `content` via a content PATCH (the whole-body write the blocking policy
/// cuts along the heading structure).
async fn patch_content(app: &common::TestApp, token: &str, resource: Uuid, content: &str) {
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "content": content }))
        .send()
        .await
        .expect("content PATCH");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "fixture: content PATCH must land; got {}",
        resp.text().await.unwrap_or_default()
    );
}

/// A resource made through the real create path (`POST /api/resources`), owned by a third
/// profile that is neither the operator nor the non-operator, given a body and then a rewrite,
/// so it carries a folded block with history beside its live blocks.
async fn create_resource_with_history(app: &common::TestApp) -> Uuid {
    let email = format!("scrub-owner-{}@example.com", Uuid::new_v4());
    let (owner, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{owner}"), &email);
    let created: Value = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://scrub-door-{}", Uuid::new_v4()),
            "title": "Door Scrub Subject",
            "slug": null
        }))
        .send()
        .await
        .expect("create request")
        .json()
        .await
        .expect("create JSON");
    let resource = Uuid::parse_str(created["id"].as_str().expect("id field")).expect("resource id");
    patch_content(app, &token, resource, BODY_A_B).await;
    patch_content(app, &token, resource, &body_a_with_filler()).await;
    resource
}

/// Every block of the resource, live and folded, in seq order — read straight off the pool.
async fn blocks_of(pool: &PgPool, resource: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY seq, id")
        .bind(resource)
        .fetch_all(pool)
        .await
        .expect("the resource's blocks")
}

/// The resource's folded blocks.
async fn folded_blocks_of(pool: &PgPool, resource: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND is_folded ORDER BY seq, id",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("the resource's folded blocks")
}

/// How many of the resource's revision bodies still carry bytes: what a scrub empties.
async fn nonempty_revisions(pool: &PgPool, resource: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_content bc \
           JOIN kb_block_revisions br ON br.id = bc.block_revision_id \
           JOIN kb_content_blocks b ON b.id = br.block_id \
          WHERE b.resource_id = $1 AND bc.content <> ''",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("non-empty revision count")
}

/// How many revision bodies of the resource's FOLDED blocks still carry bytes.
async fn nonempty_folded_revisions(pool: &PgPool, resource: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_content bc \
           JOIN kb_block_revisions br ON br.id = bc.block_revision_id \
           JOIN kb_content_blocks b ON b.id = br.block_id \
          WHERE b.resource_id = $1 AND b.is_folded AND bc.content <> ''",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("non-empty folded revision count")
}

/// What the scrub would empty of `block`, measured straight off the pool with the plan's own
/// predicates (`block_history_scrub_plan`, migration 20261003000210): revisions with bytes that
/// are not the live block's current one (every revision of a folded block), and chunks that are
/// not current (every chunk of a folded block) carrying prose, a header path or an embedding.
async fn measured_to_empty(pool: &PgPool, resource: Uuid, block: Uuid) -> (i64, i64) {
    sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM kb_block_revisions br \
              JOIN kb_block_content bc ON bc.block_revision_id = br.id \
             WHERE br.block_id = b.id AND bc.content <> '' \
               AND (b.is_folded OR br.id IS DISTINCT FROM b.current_revision_id)), \
           (SELECT count(*) FROM kb_chunks c \
             WHERE c.block_id = b.id AND c.resource_id = $1 \
               AND (b.is_folded OR NOT c.is_current) \
               AND (c.header_path IS NOT NULL OR c.embedding IS NOT NULL \
                    OR EXISTS (SELECT 1 FROM kb_chunk_content cc \
                                WHERE cc.chunk_id = c.id AND cc.content <> ''))) \
           FROM kb_content_blocks b WHERE b.id = $2",
    )
    .bind(resource)
    .bind(block)
    .fetch_one(pool)
    .await
    .expect("measured counts")
}

/// A resource with history whose fixture shape is verified: at least one folded block whose
/// revisions still carry bytes, so a scrub has something to empty (a vacuous scrub proves
/// nothing). Not every folded block carries bytes: the create-with-no-body genesis block's
/// revision is already empty when a rewrite folds it.
async fn subject(app: &common::TestApp) -> (Uuid, Vec<Uuid>) {
    let resource = create_resource_with_history(app).await;
    let blocks = blocks_of(&app.pool, resource).await;
    assert!(blocks.len() >= 2, "fixture: the resource has blocks");
    assert!(
        nonempty_folded_revisions(&app.pool, resource).await > 0,
        "fixture: a folded block's revisions carry bytes"
    );
    (resource, blocks)
}

async fn post(app: &common::TestApp, token: &str, path: &str, body: &Value) -> reqwest::Response {
    app.client
        .post(app.url(path))
        .header("Authorization", format!("Bearer {token}"))
        .json(body)
        .send()
        .await
        .expect("the door answers")
}

const EXECUTE: &str = "/api/admin/resources/block-history-scrub";
const SURVEY: &str = "/api/admin/resources/block-history-scrub/survey";
const ERASE: &str = "/api/admin/resources/erasure";

async fn count_events(pool: &PgPool, event_type: Option<&str>) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE $1::text IS NULL OR t.name = $1",
    )
    .bind(event_type)
    .fetch_one(pool)
    .await
    .expect("event count")
}

/// The ONE event of `event_type`: its id, payload and correlation id.
async fn the_event(pool: &PgPool, event_type: &str) -> (Uuid, Value, Uuid) {
    sqlx::query_as(
        "SELECT e.id, e.payload, e.correlation_id FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id WHERE t.name = $1",
    )
    .bind(event_type)
    .fetch_one(pool)
    .await
    .expect("the one event")
}

/// Erase `resource` through the operator's real erasure door.
async fn erase(app: &common::TestApp, op_token: &str, resource: Uuid) {
    let resp = post(app, op_token, ERASE, &json!({ "resource": resource })).await;
    assert_eq!(resp.status().as_u16(), 200, "fixture: the erasure lands");
    let body: Value = resp.json().await.expect("the erasure outcome");
    assert_eq!(body["status"], "completed", "fixture: {body}");
}

/// Make `resource` a cogmap's charter (its telos) — the act's own charter predicate.
async fn make_charter(pool: &PgPool, resource: Uuid) {
    sqlx::query("INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2)")
        .bind(format!("scrub-charter-{resource}"))
        .bind(resource)
        .execute(pool)
        .await
        .expect("fixture: the charter cogmap");
}

fn uuids(v: &[Uuid]) -> Value {
    Value::Array(v.iter().map(|u| Value::String(u.to_string())).collect())
}

// ── WITNESS: the operator completes, and the record equals the survey's prediction ───────────

/// FAILS IF the door does not reach the service, the response loses the reference the operator
/// must cite or the event id, the recorded correlation is not that reference, the act's
/// recorded targets diverge from the survey's per-block counts (exact prose, operator's order),
/// `cancelled_ingest` reads true on a complete ingest, or any folded block's history is left
/// carrying bytes (the fixture guarantees at least one did).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_operator_scrub_completes_and_its_targets_equal_the_surveys_counts(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, operator) = provision_operator(&app, "bhs-operator", "bhs-op@example.com").await;
    let (resource, blocks) = subject(&app).await;
    let request = json!({ "resource": resource, "blocks": blocks });
    let folded_bytes_before = nonempty_folded_revisions(&app.pool, resource).await;

    let surveyed = post(&app, &token, SURVEY, &request).await;
    assert_eq!(surveyed.status().as_u16(), 200);
    let survey: Value = surveyed.json().await.expect("the survey body");
    assert!(survey["refusal"].is_null(), "{survey}");
    let plan = &survey["plan"];
    let predicted: Vec<Value> = plan["blocks"]
        .as_array()
        .expect("per-block rows")
        .iter()
        .map(|b| {
            let state = if b["folded"].as_bool().expect("folded") {
                "folded"
            } else {
                "live"
            };
            json!({
                "target": "kb_content_blocks",
                "outcome": format!(
                    "block {} {state}: {} revisions emptied, {} chunks emptied",
                    b["block"].as_str().expect("block id"),
                    b["revisions_to_empty"],
                    b["chunks_to_empty"],
                ),
            })
        })
        .collect();
    assert_eq!(predicted.len(), blocks.len(), "one row per named block");

    let resp = post(&app, &token, EXECUTE, &request).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(body["status"], "completed", "{body}");
    assert_eq!(body["cancelled_ingest"], false, "{body}");

    let (event_id, payload, correlation) = the_event(&app.pool, "block_history_scrubbed").await;
    assert_eq!(payload["actor"], Value::String(operator.to_string()));
    assert_eq!(body["event_id"], Value::String(event_id.to_string()));
    assert_eq!(
        body["request_reference"],
        Value::String(correlation.to_string()),
        "the reference the door returns IS the recorded correlation id"
    );
    assert_eq!(
        payload["targets"],
        Value::Array(predicted.clone()),
        "the act's recorded targets must equal the survey's counts"
    );
    assert_eq!(
        body["targets"],
        Value::Array(predicted),
        "and so must the door's answer"
    );

    assert!(
        folded_bytes_before > 0,
        "fixture: a folded block carried bytes, so the emptying below is not vacuous"
    );
    assert_eq!(
        nonempty_folded_revisions(&app.pool, resource).await,
        0,
        "every scrubbed folded block empties entirely"
    );
}

// ── WITNESS: the survey records nothing ──────────────────────────────────────────────────────

/// FAILS IF the survey records any event or empties any byte, loses the operator's order, or
/// answers a folded block with history as having nothing to empty.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_survey_answers_per_block_counts_and_records_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "bhs-surveyor", "bhs-surveyor@example.com").await;
    let (resource, mut blocks) = subject(&app).await;
    blocks.reverse();
    let folded = folded_blocks_of(&app.pool, resource).await;
    let events_before = count_events(&app.pool, None).await;
    let bytes_before = nonempty_revisions(&app.pool, resource).await;

    let resp = post(
        &app,
        &token,
        SURVEY,
        &json!({ "resource": resource, "blocks": blocks }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let survey: Value = resp.json().await.expect("the survey body");
    assert!(survey["refusal"].is_null(), "{survey}");
    assert!(survey["detail"].is_null(), "{survey}");
    assert_eq!(survey["plan"]["cancels_ingest"], false, "{survey}");
    let rows = survey["plan"]["blocks"].as_array().expect("per-block rows");
    let order: Vec<Uuid> = rows
        .iter()
        .map(|r| Uuid::parse_str(r["block"].as_str().expect("block")).expect("uuid"))
        .collect();
    assert_eq!(
        order, blocks,
        "one row per named block, in the operator's order"
    );
    let mut folded_with_history = 0;
    for row in rows {
        let id = Uuid::parse_str(row["block"].as_str().expect("block")).expect("uuid");
        assert_eq!(row["folded"], folded.contains(&id), "{row}");
        let revisions = row["revisions_to_empty"].as_i64().expect("revision count");
        let chunks = row["chunks_to_empty"].as_i64().expect("chunk count");
        assert_eq!(
            (revisions, chunks),
            measured_to_empty(&app.pool, resource, id).await,
            "the survey's counts equal the rows measured on the pool: {row}"
        );
        if folded.contains(&id) && revisions > 0 {
            folded_with_history += 1;
        }
    }
    assert!(
        folded_with_history > 0,
        "at least one folded block reports history to empty: {survey}"
    );

    assert_eq!(
        count_events(&app.pool, None).await,
        events_before,
        "a survey records NOTHING"
    );
    assert_eq!(
        nonempty_revisions(&app.pool, resource).await,
        bytes_before,
        "a survey empties nothing"
    );
}

// ── WITNESS: the refused scrub of an erased resource names its act and blocks ────────────────

/// FAILS IF a scrub of an erased resource answers anything but 200 `refused` / `already_erased`,
/// the reference and event id are not the recorded refusal's, or the recorded payload does not
/// name the scrub (`act`) and the operator's blocks (an erasure refusal carries neither). The
/// survey of the same resource answers the same refusal with no plan, recording nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_scrub_of_an_erased_resource_is_a_recorded_refusal_naming_the_act_and_blocks(
    pool: PgPool,
) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "bhs-erased-op", "bhs-erased-op@example.com").await;
    let (resource, blocks) = subject(&app).await;
    erase(&app, &token, resource).await;
    let request = json!({ "resource": resource, "blocks": blocks });

    let before = count_events(&app.pool, None).await;
    let resp = post(&app, &token, SURVEY, &request).await;
    assert_eq!(resp.status().as_u16(), 200);
    let survey: Value = resp.json().await.expect("the survey body");
    assert_eq!(survey["refusal"], "already_erased", "{survey}");
    assert!(
        survey["plan"].is_null(),
        "no per-block rows on a refusal: {survey}"
    );
    assert_eq!(
        count_events(&app.pool, None).await,
        before,
        "a survey records nothing"
    );

    let resp = post(&app, &token, EXECUTE, &request).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "a recorded refusal renders 200"
    );
    let answer: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(answer["status"], "refused", "{answer}");
    assert_eq!(answer["reason"], "already_erased", "{answer}");
    assert_eq!(
        answer["blocks"],
        uuids(&blocks),
        "the refusal names the request's blocks"
    );

    let (event_id, payload, correlation) = the_event(&app.pool, "resource_erasure_refused").await;
    assert_eq!(answer["event_id"], Value::String(event_id.to_string()));
    assert_eq!(
        answer["request_reference"],
        Value::String(correlation.to_string()),
        "the reference the door returns IS the refusal's correlation id"
    );
    assert_eq!(payload["reason"], "already_erased");
    assert_eq!(payload["act"], "block_history_scrub", "{payload}");
    assert_eq!(payload["blocks"], uuids(&blocks), "{payload}");
    assert_eq!(
        count_events(&app.pool, Some("block_history_scrubbed")).await,
        0
    );
}

// ── WITNESS: a charter is refused with the map-grain detail ──────────────────────────────────

/// FAILS IF a scrub of a charter answers anything but 200 `refused` / `charter_resource` with
/// the map-grain task as its detail, records it without the scrub's act and blocks, or scrubs
/// anything. The survey answers the same refusal and detail with no plan.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_scrub_of_a_charter_is_refused_with_the_map_grain_detail(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "bhs-charter-op", "bhs-charter-op@example.com").await;
    let (resource, blocks) = subject(&app).await;
    make_charter(&app.pool, resource).await;
    let request = json!({ "resource": resource, "blocks": blocks });
    let bytes_before = nonempty_revisions(&app.pool, resource).await;

    let resp = post(&app, &token, SURVEY, &request).await;
    assert_eq!(resp.status().as_u16(), 200);
    let survey: Value = resp.json().await.expect("the survey body");
    assert_eq!(survey["refusal"], "charter_resource", "{survey}");
    assert!(
        survey["detail"]
            .as_str()
            .is_some_and(|d| d.contains("01a0e960-0ca2-7f42-b33e-1ed19b024e6b")),
        "{survey}"
    );
    assert!(survey["plan"].is_null(), "{survey}");

    let resp = post(&app, &token, EXECUTE, &request).await;
    assert_eq!(resp.status().as_u16(), 200);
    let answer: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(answer["status"], "refused", "{answer}");
    assert_eq!(answer["reason"], "charter_resource", "{answer}");
    assert_eq!(
        answer["blocks"],
        uuids(&blocks),
        "the refusal names the request's blocks"
    );
    assert!(
        answer["detail"]
            .as_str()
            .is_some_and(|d| d.contains("01a0e960-0ca2-7f42-b33e-1ed19b024e6b")),
        "{answer}"
    );
    assert!(
        !answer.to_string().contains("block_history_scrub_execute"),
        "the raise text never reaches a response: {answer}"
    );

    let (_, payload, _) = the_event(&app.pool, "resource_erasure_refused").await;
    assert_eq!(payload["reason"], "charter_resource");
    assert_eq!(payload["act"], "block_history_scrub", "{payload}");
    assert_eq!(payload["blocks"], uuids(&blocks), "{payload}");
    assert_eq!(
        count_events(&app.pool, Some("block_history_scrubbed")).await,
        0
    );
    assert_eq!(
        nonempty_revisions(&app.pool, resource).await,
        bytes_before,
        "a refused scrub empties nothing"
    );
}

// ── WITNESS: a malformed list is a 400 at both doors, recording and emptying nothing ─────────

/// FAILS IF an empty list, a repeated block, or a block of another resource answers anything
/// but 400 at either door, records any event (not even a refusal), or empties any byte; or if
/// the foreign block's 400 does not name it. The bite: the same operator scrubbing the same
/// resource with a well-formed list completes, so each 400 was about the list.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_empty_repeated_or_foreign_block_list_is_400_and_records_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "bhs-list-op", "bhs-list-op@example.com").await;
    let (resource, blocks) = subject(&app).await;
    let (other, other_blocks) = subject(&app).await;
    let foreign = other_blocks[0];
    let events_before = count_events(&app.pool, None).await;
    let bytes_before = nonempty_revisions(&app.pool, resource).await;
    let other_bytes_before = nonempty_revisions(&app.pool, other).await;

    let cases = [
        ("empty", json!({ "resource": resource, "blocks": [] })),
        (
            "repeated",
            json!({ "resource": resource, "blocks": [blocks[0], blocks[1], blocks[0]] }),
        ),
        (
            "foreign",
            json!({ "resource": resource, "blocks": [blocks[0], foreign] }),
        ),
    ];
    for door in [SURVEY, EXECUTE] {
        for (label, body) in &cases {
            let resp = post(&app, &token, door, body).await;
            assert_eq!(resp.status().as_u16(), 400, "{label} list at {door}");
            let err: Value = resp.json().await.expect("the 400 body");
            let message = err["error"]["message"].as_str().expect("message");
            assert!(
                !message.contains("block_history_scrub_execute"),
                "the raise text never reaches a response: {message}"
            );
            if *label == "foreign" {
                assert!(
                    message.contains(&foreign.to_string()),
                    "the 400 names the operator's own foreign block: {message}"
                );
            }
            if *label == "repeated" {
                assert!(message.contains(&blocks[0].to_string()), "{message}");
            }
        }
    }

    assert_eq!(
        count_events(&app.pool, None).await,
        events_before,
        "a refused list records NOTHING — no refusal event, no scrub"
    );
    assert_eq!(nonempty_revisions(&app.pool, resource).await, bytes_before);
    assert_eq!(
        nonempty_revisions(&app.pool, other).await,
        other_bytes_before
    );

    // THE BITE: the same resource, a well-formed list.
    let resp = post(
        &app,
        &token,
        EXECUTE,
        &json!({ "resource": resource, "blocks": blocks }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let answer: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(answer["status"], "completed", "{answer}");
    assert!(nonempty_revisions(&app.pool, resource).await < bytes_before);
}

// ── WITNESS: membership is checked before any refusal is recorded ────────────────────────────

/// FAILS IF a list naming a block of another resource, or an id that is no block at all, answers
/// anything but 400 at either door when the named resource is erased or a charter, or records any
/// event: the refusal ledger is never redacted, so a refusal must only ever name real blocks of
/// the resource. The bite: the same operator naming only the resource's own blocks gets the
/// recorded refusal, so each 400 was about membership, not the resource's state.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_foreign_block_on_an_erased_or_charter_resource_is_400_and_records_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) =
        provision_operator(&app, "bhs-refuse-list-op", "bhs-refuse-list-op@example.com").await;
    let (erased, erased_blocks) = subject(&app).await;
    erase(&app, &token, erased).await;
    let (charter, charter_blocks) = subject(&app).await;
    make_charter(&app.pool, charter).await;
    let (_, other_blocks) = subject(&app).await;
    let foreign = other_blocks[0];
    let no_block = Uuid::now_v7();
    let events_before = count_events(&app.pool, None).await;

    for (label, resource, own) in [
        ("erased", erased, &erased_blocks),
        ("charter", charter, &charter_blocks),
    ] {
        for (kind, stray) in [
            ("another resource's block", foreign),
            ("no block", no_block),
        ] {
            let body = json!({ "resource": resource, "blocks": [own[0], stray] });
            for door in [SURVEY, EXECUTE] {
                let resp = post(&app, &token, door, &body).await;
                assert_eq!(
                    resp.status().as_u16(),
                    400,
                    "{kind} on the {label} resource at {door}"
                );
                let err: Value = resp.json().await.expect("the 400 body");
                let message = err["error"]["message"].as_str().expect("message");
                assert!(
                    message.contains(&stray.to_string()),
                    "the 400 names the operator's own stray id: {message}"
                );
            }
        }
    }
    assert_eq!(
        count_events(&app.pool, None).await,
        events_before,
        "a list naming an id that is not a block of the resource records NOTHING, whatever the \
         resource's state"
    );

    // THE BITE: the resource's own blocks reach the recorded refusal.
    for (resource, own, reason) in [
        (erased, &erased_blocks, "already_erased"),
        (charter, &charter_blocks, "charter_resource"),
    ] {
        let resp = post(
            &app,
            &token,
            EXECUTE,
            &json!({ "resource": resource, "blocks": own }),
        )
        .await;
        assert_eq!(resp.status().as_u16(), 200);
        let answer: Value = resp.json().await.expect("the tagged outcome");
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["reason"], reason, "{answer}");
    }
    assert_eq!(
        count_events(&app.pool, Some("resource_erasure_refused")).await,
        2,
        "the two well-formed refusals are recorded"
    );
}

// ── WITNESS: an unknown field is refused at the door ─────────────────────────────────────────

/// FAILS IF either door accepts a body carrying an unknown field (a caller-chosen
/// `request_reference` would merge two acts' replay spans), or anything is recorded or emptied.
/// The bite: the same body without the field completes, and its reference is the server's.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_field_is_refused_at_the_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "bhs-ref-op", "bhs-ref-op@example.com").await;
    let (resource, blocks) = subject(&app).await;
    let chosen = Uuid::now_v7();
    let events_before = count_events(&app.pool, None).await;
    let bytes_before = nonempty_revisions(&app.pool, resource).await;

    let body = json!({ "resource": resource, "blocks": blocks, "request_reference": chosen });
    for door in [SURVEY, EXECUTE] {
        let resp = post(&app, &token, door, &body).await;
        assert_eq!(
            resp.status().as_u16(),
            422,
            "an unknown field is refused at {door}"
        );
    }
    assert_eq!(
        count_events(&app.pool, None).await,
        events_before,
        "nothing recorded"
    );
    assert_eq!(nonempty_revisions(&app.pool, resource).await, bytes_before);

    let resp = post(
        &app,
        &token,
        EXECUTE,
        &json!({ "resource": resource, "blocks": blocks }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let answer: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(answer["status"], "completed", "{answer}");
    assert_ne!(
        answer["request_reference"],
        Value::String(chosen.to_string()),
        "the reference is the server's"
    );
}

// ── WITNESS: an operator's unknown id is 404 on both doors ───────────────────────────────────

/// FAILS IF either door answers an operator's unknown resource with anything but the lookup's
/// 404 (a 500 from a raised `not found`, or a recorded refusal rendered 200), or records
/// anything.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_resource_is_404_on_both_doors(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "bhs-ghost-op", "bhs-ghost-op@example.com").await;
    let ghost = json!({ "resource": Uuid::now_v7(), "blocks": [Uuid::now_v7()] });
    let before = count_events(&app.pool, None).await;

    for door in [SURVEY, EXECUTE] {
        let resp = post(&app, &token, door, &ghost).await;
        assert_eq!(resp.status().as_u16(), 404, "unknown id at {door}");
        let body: Value = resp.json().await.expect("the 404 body");
        assert_eq!(
            body["error"]["message"], "resource not found",
            "past the gate, the lookup answers"
        );
    }
    assert_eq!(count_events(&app.pool, None).await, before);
}

// ── WITNESS: the non-operator's identical 404 at the wire, zero events, and the bite ─────────

/// FAILS IF a non-operator gets anything but the gate's own 404 body on either door, for an
/// existing, an unknown or an erased id alike — the bodies must be IDENTICAL, so the 404 says
/// neither that the resource exists nor that it was erased — or if any attempt records an
/// event or empties a byte. A list the service would refuse (empty) gets the same 404: the gate
/// answers before the service sees the list. The bite: the same caller with the gate granted
/// completes the scrub of the existing resource, so the 404 was the gate's.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_operator_gets_an_identical_404_for_existing_unknown_and_erased_ids(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, non_admin) =
        provision_non_operator(&app, "bhs-nonadmin", "bhs-nonadmin@example.com").await;
    let (op_token, _) = provision_operator(&app, "bhs-na-op", "bhs-na-op@example.com").await;
    let (existing, existing_blocks) = subject(&app).await;
    let (erased, erased_blocks) = subject(&app).await;
    erase(&app, &op_token, erased).await;
    let events_before = count_events(&app.pool, None).await;
    let bytes_before = nonempty_revisions(&app.pool, existing).await;

    let bodies = [
        json!({ "resource": existing, "blocks": existing_blocks }),
        json!({ "resource": Uuid::now_v7(), "blocks": [Uuid::now_v7()] }),
        json!({ "resource": erased, "blocks": erased_blocks }),
        json!({ "resource": existing, "blocks": [] }),
    ];
    let mut faces: Vec<Value> = Vec::new();
    for door in [SURVEY, EXECUTE] {
        for body in &bodies {
            let resp = post(&app, &token, door, body).await;
            assert_eq!(resp.status().as_u16(), 404, "{door} {body}");
            faces.push(resp.json().await.expect("the 404 body"));
        }
    }
    assert_eq!(faces[0]["error"]["message"], "not found", "the gate's face");
    assert!(
        faces.iter().all(|f| *f == faces[0]),
        "every id gets the IDENTICAL 404 body: {faces:?}"
    );
    assert_eq!(
        count_events(&app.pool, None).await,
        events_before,
        "a rejected caller appends NOTHING — no refusal, no scrub, no event at all"
    );
    assert_eq!(nonempty_revisions(&app.pool, existing).await, bytes_before);

    // THE BITE: only `is_system_admin` moves.
    temper_services::test_support::grant_governance(&app.pool, non_admin).await;
    let resp = post(&app, &token, EXECUTE, &bodies[0]).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "with the gate stood down it completes"
    );
    let answer: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(answer["status"], "completed", "{answer}");
}

// ── WITNESS: a scrub cancels an in-flight ingest, read from the recorded payload ─────────────

/// The first segment of a segmented ingest, pre-chunked and pre-embedded with the REAL chunker
/// hash (`segments_handler_test.rs`'s `one_chunk_packed`), so the begin runs without ONNX.
const SEG1: &str = "# One\n\nfirst segment";

fn one_chunk_packed(text: &str) -> String {
    let c = &temper_ingest::chunk::chunk_markdown(text)[0];
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: c.header_path.clone(),
        heading_depth: c.heading_depth,
        content: c.content.clone(),
        content_hash: c.content_hash.clone(),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

/// Begin a segmented ingest over HTTP (one landed block of a hinted two) and leave it
/// `in_progress` — `segments_handler_test.rs`'s `begin_then_cancel`, without the cancel: the
/// scrub does it. Returns the resource and its owner's token.
async fn begin_in_flight_ingest(app: &common::TestApp) -> (Uuid, String) {
    let email = format!("scrub-ingest-{}@example.com", Uuid::new_v4());
    let (owner, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{owner}"), &email);
    let begin_payload = IngestPayload {
        idempotency_key: None,
        title: "In-Flight Doc".to_string(),
        origin_uri: format!("test://scrub-in-flight-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        goal: None,
        content_hash: None,
        content: SEG1.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(one_chunk_packed(SEG1)),
        sources: Vec::new(),
        act: Default::default(),
        segmented: Some(SegmentedBegin {
            total_blocks_hint: Some(2),
            block_budget: 262_144,
            source_hash: Some("deadbeef".to_string()),
        }),
    };
    let begin: SegmentedBeginResponse = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&begin_payload)
        .send()
        .await
        .expect("begin request")
        .json()
        .await
        .expect("begin JSON");
    (begin.resource_id, token)
}

async fn ingest_state(pool: &PgPool, resource: Uuid) -> String {
    sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("ingest_state")
}

/// FAILS IF a scrub of a resource with an in-flight segmented ingest answers anything but 200
/// `completed` with `cancelled_ingest: true`, leaves `ingest_state` anything but `cancelled`, or
/// answers a value that disagrees with the recorded payload's `cancelled_ingest` (the field the
/// door reads); or if the owner's `show` of the resource does not read `ingest_state:
/// in_progress` with `ingest_ended: cancelled` (the wire ruling: an ended ingest is not whole,
/// and the new field names why). The counterpart — a complete ingest answers `false` — is
/// `an_operator_scrub_completes_and_its_targets_equal_the_surveys_counts`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_scrub_of_an_in_flight_ingest_answers_cancelled_ingest_true(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) =
        provision_operator(&app, "bhs-inflight-op", "bhs-inflight-op@example.com").await;
    let (resource, owner_token) = begin_in_flight_ingest(&app).await;
    assert_eq!(
        ingest_state(&app.pool, resource).await,
        "in_progress",
        "fixture: the ingest is in flight"
    );
    let blocks = blocks_of(&app.pool, resource).await;
    assert!(!blocks.is_empty(), "fixture: the begin landed a block");

    let survey: Value = post(
        &app,
        &token,
        SURVEY,
        &json!({ "resource": resource, "blocks": blocks }),
    )
    .await
    .json()
    .await
    .expect("the survey body");
    assert_eq!(survey["plan"]["cancels_ingest"], true, "{survey}");

    let resp = post(
        &app,
        &token,
        EXECUTE,
        &json!({ "resource": resource, "blocks": blocks }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let answer: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(answer["status"], "completed", "{answer}");
    assert_eq!(answer["cancelled_ingest"], true, "{answer}");

    let (_, payload, _) = the_event(&app.pool, "block_history_scrubbed").await;
    assert_eq!(payload["cancelled_ingest"], true, "{payload}");
    assert_eq!(
        ingest_state(&app.pool, resource).await,
        "cancelled",
        "the scrub cancelled the ingest"
    );

    let show = app
        .client
        .get(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {owner_token}"))
        .send()
        .await
        .expect("show request");
    assert_eq!(show.status().as_u16(), 200, "the owner reads the resource");
    let shown: Value = show.json().await.expect("the show body");
    assert_eq!(shown["ingest_state"], "in_progress", "{shown}");
    assert_eq!(shown["ingest_ended"], "cancelled", "{shown}");
}
