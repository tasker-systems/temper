#![cfg(feature = "test-db")]
//! Witnesses for the field scrub's operator doors (field-grain scrub spec 2026-10-09, S1, S2, S4,
//! S5; plan Task 7):
//!
//! * the execute door (`POST /api/admin/resources/field-scrub`) — an operator's keep-mode scrub
//!   completes, and the record's `redacted_fields` equal the survey's plan; an erased resource, a
//!   charter, a sentinel collision and a projection that disagrees are recorded refusals whose
//!   payload names the field scrub (`act`); a foreign handle, `properties` with `clear`, a handle
//!   with `title`, `property` without one and nothing prior are 400s that record nothing (the
//!   handle on an erased or charter resource too: it is checked before any refusal); an unknown
//!   body field is axum's 422.
//! * the survey door (`…/field-scrub/survey`) and the listing door (`…/field-scrub/families`) —
//!   they record nothing; the survey answers the refusal the act would record.
//! * a non-operator gets the gate's identical 404 on all three doors, and the ledger gains nothing.
//! * spec witness 4: distinctive secret strings seeded as a property's key text and values appear
//!   in no listing, survey, completed or refused body, no recorded `resource_scrubbed` or
//!   `resource_erasure_refused` payload, and no line the server logs while it answers them.
//!
//! The setup is `admin_block_history_scrub_surface_test.rs`'s: resources come from the real create
//! and PATCH doors. Properties are set through the SQL write path the act's own clear mode uses
//! (`property_set`, emitted by the resource's creator), so the key text is any string the test
//! chooses.

mod common;

use std::io;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

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

const TITLE: &str = "Door Field Subject";
const LATER_TITLE: &str = "A later title";

/// A resource made through the real create door (`POST /api/resources`), owned by a third profile
/// that is neither the operator nor the non-operator. Returns the resource and its owner's token.
async fn create_resource(app: &common::TestApp) -> (Uuid, String) {
    let email = format!("field-scrub-owner-{}@example.com", Uuid::new_v4());
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
            "origin_uri": format!("test://field-scrub-door-{}", Uuid::new_v4()),
            "title": TITLE,
            "slug": null
        }))
        .send()
        .await
        .expect("create request")
        .json()
        .await
        .expect("create JSON");
    let resource = Uuid::parse_str(created["id"].as_str().expect("id field")).expect("resource id");
    (resource, token)
}

/// Retitle `resource` through the owner's real PATCH door.
async fn retitle(app: &common::TestApp, owner_token: &str, resource: Uuid, title: &str) {
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {owner_token}"))
        .json(&json!({ "title": title }))
        .send()
        .await
        .expect("retitle request");
    assert_eq!(resp.status().as_u16(), 200, "fixture: the retitle lands");
}

/// A resource whose title has a prior value: created, then retitled.
async fn subject(app: &common::TestApp) -> Uuid {
    let (resource, owner_token) = create_resource(app).await;
    retitle(app, &owner_token, resource, LATER_TITLE).await;
    resource
}

/// The entity that emitted `resource`'s `resource_created`: its owner's emitter.
async fn owner_emitter(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.emitter_entity_id FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND e.payload->>'resource_id' = $1::text",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("the resource's creator")
}

/// The id of `resource`'s `resource_created` event.
async fn created_event(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND e.payload->>'resource_id' = $1::text",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("the resource's resource_created")
}

/// `resource`'s events carrying its title (its create and each retitle), in walk order.
async fn title_events(pool: &PgPool, resource: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name IN ('resource_created', 'resource_updated') \
            AND e.payload->>'resource_id' = $1::text AND e.payload ? 'title' \
          ORDER BY e.id",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("the title's events")
}

/// Set a resource-owned property through the SQL write path (`property_set`, the call the act's
/// clear mode makes for `doc_type`), as the resource's owner.
async fn set_property(pool: &PgPool, resource: Uuid, key: &str, value: &str) {
    let emitter = owner_emitter(pool, resource).await;
    sqlx::query(
        "SELECT property_set(jsonb_build_object(\
             'property_id',  uuid_generate_v7(), \
             'owner',        jsonb_build_object('table', 'kb_resources', 'id', $1::uuid), \
             'property_key', $2::text, \
             'value',        to_jsonb($3::text), \
             'weight',       1.0), $4)",
    )
    .bind(resource)
    .bind(key)
    .bind(value)
    .bind(emitter)
    .execute(pool)
    .await
    .expect("fixture: the property is set");
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

/// POST and read the body as text, asserting the status.
async fn post_text(
    app: &common::TestApp,
    token: &str,
    path: &str,
    body: &Value,
    status: u16,
) -> String {
    let resp = post(app, token, path, body).await;
    let got = resp.status().as_u16();
    let text = resp.text().await.expect("the body");
    assert_eq!(got, status, "{path} {body}: {text}");
    text
}

const EXECUTE: &str = "/api/admin/resources/field-scrub";
const SURVEY: &str = "/api/admin/resources/field-scrub/survey";
const FAMILIES: &str = "/api/admin/resources/field-scrub/families";
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

async fn payload_of(pool: &PgPool, event: Uuid) -> Value {
    sqlx::query_scalar("SELECT payload FROM kb_events WHERE id = $1")
        .bind(event)
        .fetch_one(pool)
        .await
        .expect("the event's payload")
}

async fn title_of(pool: &PgPool, resource: Uuid) -> String {
    sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("the title")
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
        .bind(format!("field-scrub-charter-{resource}"))
        .bind(resource)
        .execute(pool)
        .await
        .expect("fixture: the charter cogmap");
}

/// The handle the listing door gives the property family whose live row carries `key`.
async fn handle_of(app: &common::TestApp, op_token: &str, resource: Uuid, key: &str) -> Uuid {
    let handle: Uuid = sqlx::query_scalar(
        "SELECT f.family FROM resource_field_scrub_families($1) f \
          WHERE f.field = 'property' \
            AND EXISTS (SELECT 1 FROM kb_events e WHERE e.id = f.family \
                         AND e.payload->>'property_key' = $2)",
    )
    .bind(resource)
    .bind(key)
    .fetch_one(&app.pool)
    .await
    .expect("fixture: the family's handle");
    let listing: Value = post(app, op_token, FAMILIES, &json!({ "resource": resource }))
        .await
        .json()
        .await
        .expect("the listing");
    assert!(
        listing["families"]
            .as_array()
            .expect("families")
            .iter()
            .any(|f| f["family"] == Value::String(handle.to_string())),
        "the listing door names the handle: {listing}"
    );
    handle
}

/// Forge the title guard's inversion (spec witness 7's shape, committed): a `resource_updated`
/// whose id sorts just before the last one, projected, so the walk's last title is not the
/// projection's.
async fn invert_the_title_order(pool: &PgPool, resource: Uuid) {
    let last: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_updated' AND e.payload->>'resource_id' = $1::text \
            AND e.payload ? 'title' ORDER BY e.id DESC LIMIT 1",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("fixture: the last retitle");
    let inverted = Uuid::from_u128(last.as_u128() - 1);
    let payload = json!({ "resource_id": resource, "title": "an inverted title" });
    let emitter = owner_emitter(pool, resource).await;
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(
        "INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table, \
                                producing_anchor_id, payload, category) \
         SELECT $1, et.id, $2, h.anchor_table, h.anchor_id, $3, 'domain' \
           FROM kb_event_types et, kb_resource_homes h \
          WHERE et.name = 'resource_updated' AND h.resource_id = $4",
    )
    .bind(inverted)
    .bind(emitter)
    .bind(&payload)
    .bind(resource)
    .execute(&mut *tx)
    .await
    .expect("fixture: the inverted event");
    sqlx::query("SELECT _project_resource_updated($1, $2)")
        .bind(inverted)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .expect("fixture: the inverted event projects");
    tx.commit().await.expect("fixture: commit the inversion");
}

// ── WITNESS: the operator completes, and the record equals the survey's plan ─────────────────

/// FAILS IF the door does not reach the act, the response loses the reference or the event id,
/// the recorded correlation is not that reference, the record's `redacted_fields` diverge from
/// the survey's plan or the door's answer, the record names a family for `title` or says
/// `cleared`, the kept title changes, or the prior title survives on the ledger.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_operator_keep_mode_scrub_completes_and_its_record_equals_the_surveys_plan(
    pool: PgPool,
) {
    let app = common::setup_test_app(pool).await;
    let (token, operator) = provision_operator(&app, "fs-operator", "fs-op@example.com").await;
    let resource = subject(&app).await;
    let created = created_event(&app.pool, resource).await;
    let titles = title_events(&app.pool, resource).await;
    assert_eq!(
        titles.first(),
        Some(&created),
        "fixture: the create carries the title"
    );
    let prior: Vec<Value> = titles[..titles.len() - 1]
        .iter()
        .map(|e| json!({ "event": e, "paths": ["title"] }))
        .collect();
    let request = json!({ "resource": resource, "field": "title" });

    let survey: Value = serde_json::from_str(&post_text(&app, &token, SURVEY, &request, 200).await)
        .expect("the survey body");
    assert!(survey["refusal"].is_null(), "{survey}");
    let planned = survey["plan"]["redacted_fields"].clone();
    assert_eq!(
        planned,
        Value::Array(prior),
        "every title but today's is prior: {survey}"
    );
    assert!(
        survey["plan"]["clears"]
            .as_array()
            .expect("clears")
            .is_empty(),
        "keep mode appends nothing: {survey}"
    );

    let body: Value = serde_json::from_str(&post_text(&app, &token, EXECUTE, &request, 200).await)
        .expect("the tagged outcome");
    assert_eq!(body["status"], "completed", "{body}");
    assert_eq!(body["cleared"], false, "{body}");
    assert_eq!(body["field"], json!({ "kind": "title" }), "{body}");

    let (event_id, payload, correlation) = the_event(&app.pool, "resource_scrubbed").await;
    assert_eq!(payload["actor"], Value::String(operator.to_string()));
    assert_eq!(payload["subject_id"], Value::String(resource.to_string()));
    assert_eq!(payload["field"], json!({ "kind": "title" }), "{payload}");
    assert!(payload.get("cleared").is_none(), "{payload}");
    assert_eq!(body["event_id"], Value::String(event_id.to_string()));
    assert_eq!(
        body["request_reference"],
        Value::String(correlation.to_string()),
        "the reference the door returns IS the recorded correlation id"
    );
    assert_eq!(
        payload["redacted_fields"], planned,
        "the record equals the plan"
    );
    assert_eq!(
        body["redacted_fields"], planned,
        "and so does the door's answer"
    );

    assert_eq!(
        title_of(&app.pool, resource).await,
        LATER_TITLE,
        "today's title is kept"
    );
    assert_ne!(
        payload_of(&app.pool, created).await["title"],
        TITLE,
        "the prior title is gone from the ledger"
    );
}

// ── WITNESS: the listing and the survey record nothing ────────────────────────────────────────

/// FAILS IF the listing or the survey (in either mode) records any event or changes the title, or
/// the listing loses the title row or a property family's handle.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_listing_and_the_survey_record_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-surveyor", "fs-surveyor@example.com").await;
    let resource = subject(&app).await;
    set_property(&app.pool, resource, "colour", "blue").await;
    set_property(&app.pool, resource, "colour", "green").await;
    let handle = handle_of(&app, &token, resource, "colour").await;
    let events_before = count_events(&app.pool, None).await;

    let listing: Value = serde_json::from_str(
        &post_text(
            &app,
            &token,
            FAMILIES,
            &json!({ "resource": resource }),
            200,
        )
        .await,
    )
    .expect("the listing");
    let families = listing["families"].as_array().expect("families");
    assert_eq!(families[0]["field"], "title", "{listing}");
    assert_eq!(
        families[0]["events"],
        title_events(&app.pool, resource).await.len(),
        "{listing}"
    );
    for (field, family, clear) in [
        (json!("title"), Value::Null, false),
        (json!("title"), Value::Null, true),
        (json!("property"), json!(handle), false),
        (json!("property"), json!(handle), true),
        (json!("properties"), Value::Null, false),
    ] {
        let body =
            json!({ "resource": resource, "field": field, "family": family, "clear": clear });
        let survey: Value =
            serde_json::from_str(&post_text(&app, &token, SURVEY, &body, 200).await)
                .expect("the survey");
        assert!(survey["refusal"].is_null(), "{survey}");
        assert!(survey["families"].is_array(), "{survey}");
        assert_eq!(
            survey["plan"]["clears"]
                .as_array()
                .expect("clears")
                .is_empty(),
            !clear,
            "clear mode names its clearing event: {survey}"
        );
    }

    assert_eq!(
        count_events(&app.pool, None).await,
        events_before,
        "the listing and the survey record NOTHING"
    );
    assert_eq!(title_of(&app.pool, resource).await, LATER_TITLE);
}

// ── WITNESS: a refused scrub is recorded, naming the field scrub ─────────────────────────────

/// Execute `body` and assert a recorded refusal for `reason`: the door's answer, the one refusal
/// event naming the field scrub's act and no blocks, and no `resource_scrubbed`.
async fn assert_recorded_refusal(app: &common::TestApp, token: &str, body: &Value, reason: &str) {
    let answer: Value = serde_json::from_str(&post_text(app, token, EXECUTE, body, 200).await)
        .expect("the tagged outcome");
    assert_eq!(answer["status"], "refused", "{answer}");
    assert_eq!(answer["reason"], reason, "{answer}");
    assert!(
        !answer.to_string().contains("resource_field_scrub_execute"),
        "the raise text never reaches a response: {answer}"
    );
    let (event_id, payload, correlation) = the_event(&app.pool, "resource_erasure_refused").await;
    assert_eq!(answer["event_id"], Value::String(event_id.to_string()));
    assert_eq!(
        answer["request_reference"],
        Value::String(correlation.to_string())
    );
    assert_eq!(payload["reason"], reason, "{payload}");
    assert_eq!(payload["act"], "field_scrub", "{payload}");
    assert!(payload.get("blocks").is_none(), "{payload}");
    assert_eq!(count_events(&app.pool, Some("resource_scrubbed")).await, 0);
}

/// FAILS IF a scrub of an erased resource answers anything but a recorded `already_erased`
/// refusal naming the field scrub, or its survey does not answer the same refusal with no plan
/// and no listing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_scrub_of_an_erased_resource_is_a_recorded_refusal(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-erased-op", "fs-erased-op@example.com").await;
    let resource = subject(&app).await;
    erase(&app, &token, resource).await;
    let request = json!({ "resource": resource, "field": "title" });

    let before = count_events(&app.pool, None).await;
    let survey: Value = serde_json::from_str(&post_text(&app, &token, SURVEY, &request, 200).await)
        .expect("the survey");
    assert_eq!(survey["refusal"], "already_erased", "{survey}");
    assert!(survey["plan"].is_null(), "{survey}");
    assert!(survey["families"].is_null(), "{survey}");
    assert_eq!(
        count_events(&app.pool, None).await,
        before,
        "a survey records nothing"
    );

    assert_recorded_refusal(&app, &token, &request, "already_erased").await;
}

/// FAILS IF a scrub of a charter answers anything but a recorded `charter_resource` refusal with
/// the map-grain task as its detail, or changes the title.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_scrub_of_a_charter_is_refused_with_the_map_grain_detail(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-charter-op", "fs-charter-op@example.com").await;
    let resource = subject(&app).await;
    make_charter(&app.pool, resource).await;
    let request = json!({ "resource": resource, "field": "title" });

    let survey: Value = serde_json::from_str(&post_text(&app, &token, SURVEY, &request, 200).await)
        .expect("the survey");
    assert_eq!(survey["refusal"], "charter_resource", "{survey}");
    assert!(
        survey["detail"]
            .as_str()
            .is_some_and(|d| d.contains("01a0e960-0ca2-7f42-b33e-1ed19b024e6b")),
        "{survey}"
    );

    assert_recorded_refusal(&app, &token, &request, "charter_resource").await;
    let (_, payload, _) = the_event(&app.pool, "resource_erasure_refused").await;
    assert!(
        payload["detail"]
            .as_str()
            .is_some_and(|d| d.contains("01a0e960-0ca2-7f42-b33e-1ed19b024e6b")),
        "{payload}"
    );
    assert_eq!(title_of(&app.pool, resource).await, LATER_TITLE);
}

/// FAILS IF a family whose scrub sentinel the owner's ledger already names as key text is
/// scrubbed instead of refused `sentinel_collision`, or the survey does not predict it. Clear mode,
/// because only a renamed key takes the key sentinel: a live family's key text is kept in keep
/// mode.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_sentinel_collision_is_a_recorded_refusal(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-collide-op", "fs-collide-op@example.com").await;
    let (resource, _) = create_resource(&app).await;
    set_property(&app.pool, resource, "colour", "blue").await;
    set_property(&app.pool, resource, "colour", "green").await;
    let handle = handle_of(&app, &token, resource, "colour").await;
    set_property(
        &app.pool,
        resource,
        &format!("scrubbed-key-{handle}"),
        "typed in the sentinel's shape",
    )
    .await;
    let request =
        json!({ "resource": resource, "field": "property", "family": handle, "clear": true });

    let survey: Value = serde_json::from_str(&post_text(&app, &token, SURVEY, &request, 200).await)
        .expect("the survey");
    assert_eq!(survey["refusal"], "sentinel_collision", "{survey}");
    assert!(survey["plan"].is_null(), "{survey}");

    assert_recorded_refusal(&app, &token, &request, "sentinel_collision").await;
}

/// FAILS IF keep mode scrubs a title whose latest event disagrees with the projection instead of
/// refusing `projection_disagrees` (spec witness 7, at the door).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_projection_that_disagrees_is_a_recorded_refusal(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-invert-op", "fs-invert-op@example.com").await;
    let resource = subject(&app).await;
    invert_the_title_order(&app.pool, resource).await;
    let request = json!({ "resource": resource, "field": "title" });

    let survey: Value = serde_json::from_str(&post_text(&app, &token, SURVEY, &request, 200).await)
        .expect("the survey");
    assert_eq!(survey["refusal"], "projection_disagrees", "{survey}");

    assert_recorded_refusal(&app, &token, &request, "projection_disagrees").await;
}

// ── WITNESS: a malformed or foreign request is a 400 that records nothing ─────────────────────

/// FAILS IF a foreign handle (another resource's family, or this resource's event that is no
/// family handle), `properties` with `clear`, a handle with `title`, `property` without a handle,
/// or keep mode with nothing prior answers anything but 400 at the execute door, records any
/// event, or renders the raise text; or if a foreign handle's 400 does not name it. The survey
/// answers the same 400s, except nothing prior, which it reports as a plan with nothing to redact.
/// The bite: the same operator's well-formed request completes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_malformed_or_foreign_request_is_400_and_records_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-400-op", "fs-400-op@example.com").await;
    let resource = subject(&app).await;
    let (other, _) = create_resource(&app).await;
    let foreign = created_event(&app.pool, other).await;
    let retitled: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_updated' AND e.payload->>'resource_id' = $1::text LIMIT 1",
    )
    .bind(resource)
    .fetch_one(&app.pool)
    .await
    .expect("the retitle event");
    let (fresh, _) = create_resource(&app).await;
    let events_before = count_events(&app.pool, None).await;

    let cases = [
        (
            "another resource's handle",
            json!({ "resource": resource, "field": "property", "family": foreign }),
            Some(foreign),
        ),
        (
            "an event that is no family handle",
            json!({ "resource": resource, "field": "property", "family": retitled }),
            Some(retitled),
        ),
        (
            "properties with clear",
            json!({ "resource": resource, "field": "properties", "clear": true }),
            None,
        ),
        (
            "a handle with title",
            json!({ "resource": resource, "field": "title", "family": foreign }),
            None,
        ),
        (
            "property without a handle",
            json!({ "resource": resource, "field": "property" }),
            None,
        ),
    ];
    for door in [SURVEY, EXECUTE] {
        for (label, body, named) in &cases {
            let text = post_text(&app, &token, door, body, 400).await;
            assert!(
                !text.contains("resource_field_scrub"),
                "{label} at {door}: the raise text never reaches a response: {text}"
            );
            if let Some(id) = named {
                assert!(text.contains(&id.to_string()), "{label} at {door}: {text}");
            }
        }
    }

    let nothing_prior = json!({ "resource": fresh, "field": "title" });
    let text = post_text(&app, &token, EXECUTE, &nothing_prior, 400).await;
    assert!(text.contains("nothing prior"), "{text}");
    let survey: Value =
        serde_json::from_str(&post_text(&app, &token, SURVEY, &nothing_prior, 200).await)
            .expect("the survey");
    assert!(survey["refusal"].is_null(), "{survey}");
    assert_eq!(survey["plan"]["redacted_fields"], json!([]), "{survey}");

    assert_eq!(
        count_events(&app.pool, None).await,
        events_before,
        "a refused request records NOTHING — no refusal event, no scrub"
    );

    // THE BITE: the same resource, a well-formed request.
    let answer: Value = serde_json::from_str(
        &post_text(
            &app,
            &token,
            EXECUTE,
            &json!({ "resource": resource, "field": "title" }),
            200,
        )
        .await,
    )
    .expect("the tagged outcome");
    assert_eq!(answer["status"], "completed", "{answer}");
}

/// FAILS IF a handle that is not one of the resource's families reaches a recorded refusal on an
/// erased or charter resource: a refusal is recorded in a ledger nothing redacts, so it must only
/// ever name a real family of the resource (S4). The bite: the resource's own request reaches the
/// recorded refusal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_foreign_handle_on_an_erased_or_charter_resource_is_400_and_records_nothing(
    pool: PgPool,
) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-ref-op", "fs-ref-op@example.com").await;
    let erased = subject(&app).await;
    erase(&app, &token, erased).await;
    let charter = subject(&app).await;
    make_charter(&app.pool, charter).await;
    let (other, _) = create_resource(&app).await;
    let foreign = created_event(&app.pool, other).await;
    let events_before = count_events(&app.pool, None).await;

    for resource in [erased, charter] {
        let body = json!({ "resource": resource, "field": "property", "family": foreign });
        for door in [SURVEY, EXECUTE] {
            let text = post_text(&app, &token, door, &body, 400).await;
            assert!(text.contains(&foreign.to_string()), "{text}");
        }
    }
    assert_eq!(count_events(&app.pool, None).await, events_before);

    let doc_type = created_event(&app.pool, charter).await;
    let answer: Value = serde_json::from_str(
        &post_text(
            &app,
            &token,
            EXECUTE,
            &json!({ "resource": charter, "field": "property", "family": doc_type }),
            200,
        )
        .await,
    )
    .expect("the tagged outcome");
    assert_eq!(answer["reason"], "charter_resource", "{answer}");
}

// ── WITNESS: an unknown body field is refused at every door ──────────────────────────────────

/// FAILS IF any door accepts a body carrying an unknown field (a caller-chosen
/// `request_reference` would merge two acts' replay spans) or an unknown field kind, or anything
/// is recorded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_field_is_refused_at_every_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-422-op", "fs-422-op@example.com").await;
    let resource = subject(&app).await;
    let chosen = Uuid::now_v7();
    let events_before = count_events(&app.pool, None).await;

    let act = json!({ "resource": resource, "field": "title", "request_reference": chosen });
    let listing = json!({ "resource": resource, "request_reference": chosen });
    // An unknown field kind is no `ScrubFieldKind`: the body does not parse either.
    let kind = json!({ "resource": resource, "field": "body" });
    for (door, body) in [
        (SURVEY, &act),
        (EXECUTE, &act),
        (FAMILIES, &listing),
        (SURVEY, &kind),
        (EXECUTE, &kind),
    ] {
        let resp = post(&app, &token, door, body).await;
        assert_eq!(resp.status().as_u16(), 422, "an unknown field at {door}");
    }
    assert_eq!(count_events(&app.pool, None).await, events_before);
    assert_eq!(title_of(&app.pool, resource).await, LATER_TITLE);
}

// ── WITNESS: an operator's unknown id is 404 on every door ───────────────────────────────────

/// FAILS IF any door answers an operator's unknown resource with anything but the lookup's 404,
/// or records anything.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_resource_is_404_on_every_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-ghost-op", "fs-ghost-op@example.com").await;
    let ghost = Uuid::now_v7();
    let before = count_events(&app.pool, None).await;

    for (door, body) in [
        (SURVEY, json!({ "resource": ghost, "field": "title" })),
        (EXECUTE, json!({ "resource": ghost, "field": "title" })),
        (FAMILIES, json!({ "resource": ghost })),
    ] {
        let resp = post(&app, &token, door, &body).await;
        assert_eq!(resp.status().as_u16(), 404, "unknown id at {door}");
        let body: Value = resp.json().await.expect("the 404 body");
        assert_eq!(body["error"]["message"], "resource not found", "{door}");
    }
    assert_eq!(count_events(&app.pool, None).await, before);
}

// ── WITNESS: the non-operator's identical 404 at the wire, zero events, and the bite ─────────

/// FAILS IF a non-operator gets anything but the gate's own 404 body on any door, for an existing,
/// an unknown or an erased id alike — the bodies must be IDENTICAL — or if any attempt records an
/// event or changes the title. A request the service would refuse (`properties` with `clear`) gets
/// the same 404: the gate answers before the service reads the request. The bite: the same caller
/// with the gate granted completes the scrub.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_operator_gets_an_identical_404_on_every_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, non_admin) =
        provision_non_operator(&app, "fs-nonadmin", "fs-nonadmin@example.com").await;
    let (op_token, _) = provision_operator(&app, "fs-na-op", "fs-na-op@example.com").await;
    let existing = subject(&app).await;
    let erased = subject(&app).await;
    erase(&app, &op_token, erased).await;
    let events_before = count_events(&app.pool, None).await;

    let mut faces: Vec<Value> = Vec::new();
    for resource in [existing, Uuid::now_v7(), erased] {
        for (door, body) in [
            (SURVEY, json!({ "resource": resource, "field": "title" })),
            (EXECUTE, json!({ "resource": resource, "field": "title" })),
            (
                EXECUTE,
                json!({ "resource": resource, "field": "properties", "clear": true }),
            ),
            (FAMILIES, json!({ "resource": resource })),
        ] {
            let resp = post(&app, &token, door, &body).await;
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
    assert_eq!(title_of(&app.pool, existing).await, LATER_TITLE);

    // THE BITE: only `is_system_admin` moves.
    temper_services::test_support::grant_governance(&app.pool, non_admin).await;
    let answer: Value = serde_json::from_str(
        &post_text(
            &app,
            &token,
            EXECUTE,
            &json!({ "resource": existing, "field": "title" }),
            200,
        )
        .await,
    )
    .expect("the tagged outcome");
    assert_eq!(answer["status"], "completed", "{answer}");
}

// ── SPEC WITNESS 4: no text crosses ──────────────────────────────────────────────────────────

/// Captures what the server stack writes (`export_resolution_is_logged.rs`'s writer), so the
/// assertion is on the lines actually emitted.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("poisoned")).into_owned()
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Capture {
    type Writer = Self;
    fn make_writer(&'w self) -> Self::Writer {
        self.clone()
    }
}

/// The marker every seeded secret carries, so one search finds any of them.
const SECRET_MARK: &str = "sk-live-";
const SECRET_KEY: &str = "sk-live-KEY4f9a2c7e1b30d8";
const SECRET_VALUE: &str = "sk-live-VAL0d3e8b6a5c1f72";
const LATER_SECRET_VALUE: &str = "sk-live-VAL9b1c4e7d2a6f05";

/// Spec witness 4. A property family whose key text and both values are secret-shaped strings is
/// listed, surveyed in both modes and scrubbed in clear mode on one resource; on another, the same
/// family is refused `sentinel_collision`. No secret appears in any answer, in the recorded
/// `resource_scrubbed` or `resource_erasure_refused` payload, or in any line the server logged
/// while answering, at every level.
///
/// How the capture reaches the server's lines: `#[sqlx::test]` runs the test on a current-thread
/// tokio runtime, and `setup_test_app` spawns the server onto that runtime, so every handler and
/// service future polls on this thread. `tracing::subscriber::set_default` installs the capturing
/// subscriber as this thread's default for the guard's lifetime, which covers them. The capture is
/// shown to hold the server's lines by the gate's own warning and a 400's log line, both asserted
/// present.
///
/// FAILS IF any door, record or log line carries the key text or a value.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_seeded_secret_crosses_any_door_record_or_log_line(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = provision_operator(&app, "fs-secret-op", "fs-secret-op@example.com").await;
    let (non_admin, _) =
        provision_non_operator(&app, "fs-secret-na", "fs-secret-na@example.com").await;

    let (scrubbed, _) = create_resource(&app).await;
    set_property(&app.pool, scrubbed, SECRET_KEY, SECRET_VALUE).await;
    set_property(&app.pool, scrubbed, SECRET_KEY, LATER_SECRET_VALUE).await;
    let (refused, _) = create_resource(&app).await;
    set_property(&app.pool, refused, SECRET_KEY, SECRET_VALUE).await;
    set_property(&app.pool, refused, SECRET_KEY, LATER_SECRET_VALUE).await;
    let scrubbed_handle = handle_of(&app, &token, scrubbed, SECRET_KEY).await;
    let refused_handle = handle_of(&app, &token, refused, SECRET_KEY).await;
    set_property(
        &app.pool,
        refused,
        &format!("scrubbed-key-{refused_handle}"),
        "typed in the sentinel's shape",
    )
    .await;

    let captured = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let mut answers = Vec::new();
    for resource in [scrubbed, refused] {
        answers.push(
            post_text(
                &app,
                &token,
                FAMILIES,
                &json!({ "resource": resource }),
                200,
            )
            .await,
        );
    }
    for (family, clear) in [(scrubbed_handle, false), (scrubbed_handle, true)] {
        let body = json!({
            "resource": scrubbed, "field": "property", "family": family, "clear": clear,
        });
        answers.push(post_text(&app, &token, SURVEY, &body, 200).await);
    }
    answers.push(
        post_text(
            &app,
            &token,
            SURVEY,
            &json!({ "resource": scrubbed, "field": "properties" }),
            200,
        )
        .await,
    );
    let refused_body = json!({
        "resource": refused, "field": "property", "family": refused_handle, "clear": true,
    });
    answers.push(post_text(&app, &token, SURVEY, &refused_body, 200).await);
    let refusal = post_text(&app, &token, EXECUTE, &refused_body, 200).await;
    assert!(refusal.contains("sentinel_collision"), "{refusal}");
    answers.push(refusal);
    let completed = post_text(
        &app,
        &token,
        EXECUTE,
        &json!({
            "resource": scrubbed, "field": "property", "family": scrubbed_handle, "clear": true,
        }),
        200,
    )
    .await;
    assert!(completed.contains("\"completed\""), "{completed}");
    answers.push(completed);
    // Two lines the capture must hold, so its silence on the secret is not the silence of a
    // capture that saw nothing: the gate's warning, and a 400's.
    post_text(
        &app,
        &non_admin,
        FAMILIES,
        &json!({ "resource": scrubbed }),
        404,
    )
    .await;
    post_text(
        &app,
        &token,
        EXECUTE,
        &json!({ "resource": scrubbed, "field": "properties", "clear": true }),
        400,
    )
    .await;
    drop(guard);

    for answer in &answers {
        assert!(
            !answer.contains(SECRET_MARK),
            "an answer carries a secret: {answer}"
        );
    }
    let (_, scrub_record, _) = the_event(&app.pool, "resource_scrubbed").await;
    assert!(
        !scrub_record.to_string().contains(SECRET_MARK),
        "the record carries a secret: {scrub_record}"
    );
    let (_, refusal_record, _) = the_event(&app.pool, "resource_erasure_refused").await;
    assert!(
        !refusal_record.to_string().contains(SECRET_MARK),
        "the refusal carries a secret: {refusal_record}"
    );

    let log = captured.contents();
    assert!(
        log.contains("erasure door refused a caller who is not a system admin"),
        "the capture holds the server's lines: {log}"
    );
    assert!(
        log.contains("field_scrub.families"),
        "the capture holds the gate's door name: {log}"
    );
    assert!(
        log.contains("bad request"),
        "the capture holds a 400's line: {log}"
    );
    assert!(
        !log.contains(SECRET_MARK),
        "a log line carries a secret:\n{}",
        log.lines()
            .filter(|l| l.contains(SECRET_MARK))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
