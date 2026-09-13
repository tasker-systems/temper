#![cfg(feature = "test-db")]
//! Beat 4 witnesses for the erasure act's operator surfaces:
//!
//! * the execute door (`POST /api/admin/erasure`) — an operator completes an erasure through
//!   HTTP with the payload the spec requires; a non-operator gets the **404 posture** while the
//!   `unauthorized` refusal is RECORDED and nothing else mutates (ruled constraint 1: the gate
//!   lives in the service, the door renders the posture, deny is 404 never 403); and the bite
//!   probe — the same caller, the gate granted, the same request completes, proving the
//!   refusal was the gate's work and not the router's.
//! * the survey door (`POST /api/admin/erasure/survey`, task 01a09628 item 2) — a read-only
//!   preview sharing the act's computation: an operator gets the prediction (no `event_id` —
//!   nothing fired) and the ledger gained nothing; a non-operator gets the same 404 and ZERO
//!   new events (a survey attempt is not an erasure request — no refusal is recorded, ruled
//!   2026-09-12), with the same bite probe; and survey-then-execute parity — the act's
//!   recorded payload targets equal the survey's predicted targets, exactly.
//! * the audit read (`GET /api/admin/ledger`) — Beat 3 already admitted both erasure families
//!   to the admin catalogue (`admin_ledger_service::ADMIN_EVENT_TYPES`), so the Operator row's
//!   requirement ("An admin read surface lists erasures and refusals") is MET by the existing
//!   surface; these tests PIN it rather than build a duplicate door, and assert the record
//!   never re-identifies: the subject appears only as the pseudonym the act itself broke.

mod common;

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// A profile + its `<handle>@web` emitter entity (the erasure_service fixture shape: the
/// handle is the FULL id, so same-millisecond uuidv7 handles cannot collide). Returns the
/// profile's identity-bearing values so the pseudonymity assertions have something to NOT find.
async fn insert_profile(pool: &PgPool) -> (Uuid, String, String) {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    let email = format!("{handle}@x.test");
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
                 VALUES ($1, $2, $2, $3, '{\"theme\":\"dark\"}'::jsonb)",
    )
    .bind(id)
    .bind(&handle)
    .bind(&email)
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("{handle}@web"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    (id, handle, email)
}

/// Resolve the profile a test JWT's `sub` provisioned (the auth middleware's one mapping:
/// `kb_profile_auth_links.auth_provider_user_id`).
async fn profile_of_sub(pool: &PgPool, sub: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
    )
    .bind(sub)
    .fetch_one(pool)
    .await
    .expect("the provisioned profile")
}

/// Provision `sub` through a real authenticated request, then grant the operator standing.
/// A JWT first sign-in is born `Denied` (Provisioner::OauthFirstLogin), so the router's
/// system-access layer needs `approved` standing seeded explicitly, and `is_system_admin` IS a
/// governance row (D10) — `make_test_admin` seeds both.
async fn provision_and_make_operator(
    app: &common::TestApp,
    sub: &str,
    email: &str,
) -> (String, Uuid) {
    let token = common::generate_test_jwt(sub, email);
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("provisioning request");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "first sign-in must provision: {}",
        resp.text().await.unwrap_or_default()
    );
    let profile = profile_of_sub(&app.pool, sub).await;
    common::fixtures::make_test_admin(&app.pool, profile).await;
    (token, profile)
}

/// Provision `sub` and grant standing ONLY — reaches the gated router but fails the service's
/// `is_system_admin` gate: the non-operator the door must render absent.
async fn provision_non_operator(app: &common::TestApp, sub: &str, email: &str) -> (String, Uuid) {
    let token = common::generate_test_jwt(sub, email);
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("provisioning request");
    assert_eq!(resp.status().as_u16(), 200);
    let profile = profile_of_sub(&app.pool, sub).await;
    common::fixtures::approve_standing(&app.pool, profile).await;
    (token, profile)
}

/// The ONE `principal_erased` event on the ledger, with its payload and references.
async fn the_completion(app: &common::TestApp) -> (Value, Value) {
    sqlx::query_as(
        "SELECT e.payload, e.\"references\" FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'principal_erased'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("the completion event")
}

// ── WITNESS: the operator completes through HTTP ─────────────────────────────────────────────

/// FAILS IF the door does not reach the service, or the recorded payload loses the act's facts:
/// subject as the pseudonym, actor distinct, the request reference on the `request` ref and the
/// correlation, the redacted set as hashes only.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_operator_completes_an_erasure_through_the_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, operator) =
        provision_and_make_operator(&app, "erasure-operator", "operator@example.com").await;
    let (subject, _, _) = insert_profile(&app.pool).await;
    let request_reference = Uuid::now_v7();

    let resp = app
        .client
        .post(app.url("/api/admin/erasure"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "subject": subject,
            "request_reference": request_reference,
        }))
        .send()
        .await
        .expect("the door answers");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "an operator's act completes: {}",
        resp.text().await.unwrap_or_default()
    );

    // The RECORDED payload is the audit — assert its facts, not just the 200.
    let (payload, references) = the_completion(&app).await;
    assert_eq!(payload["subject_table"], "kb_profiles");
    assert_eq!(payload["subject_id"], Value::String(subject.to_string()));
    assert_eq!(payload["actor"], Value::String(operator.to_string()));
    let refs = references.as_array().expect("references array");
    assert!(
        refs.iter().any(|r| r["rel"] == "request"
            && r["target"]["kind"] == "kb_events"
            && r["target"]["id"] == Value::String(request_reference.to_string())),
        "the request reference rides the `request` ref, got {refs:?}"
    );
    let (correlation,): (Uuid,) = sqlx::query_as(
        "SELECT correlation_id FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'principal_erased'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("the completion");
    assert_eq!(
        correlation, request_reference,
        "the correlation id IS the request reference — the pairing is a fact"
    );

    // And the door SAYS what happened.
    let body: Value = resp
        .json()
        .await
        .expect("the response is the tagged outcome");
    assert_eq!(body["status"], "completed", "{body}");
    assert_eq!(body["already_erased"], false);
    assert!(
        body["targets"].is_array(),
        "the door reports the per-target outcomes — the named remainder is visible at the \
         door, not only in the ledger: {body}"
    );
}

// ── WITNESS: the non-operator's 404 posture, the recorded refusal, and the bite ──────────────

/// FAILS IF a non-operator's attempt mutates anything, leaks anything but 404, or skips the
/// recorded refusal — and, as the bite probe: FAILS IF the refusal came from anywhere but the
/// service gate, because the SAME caller with the gate granted completes the SAME request.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_operator_gets_404_and_a_recorded_refusal_until_the_gate_stands_down(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (subject, subject_handle, subject_email) = insert_profile(&app.pool).await;
    let (token, non_admin) =
        provision_non_operator(&app, "erasure-nonadmin", "nonadmin@example.com").await;
    let request_reference = Uuid::now_v7();

    let resp = app
        .client
        .post(app.url("/api/admin/erasure"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "subject": subject,
            "request_reference": request_reference,
        }))
        .send()
        .await
        .expect("the door answers");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "the door renders ABSENT to a caller the gate declined, got {}",
        resp.text().await.unwrap_or_default()
    );

    // …and the service recorded the refusal, attributed to the attempter.
    let (reason, actor): (String, Uuid) = sqlx::query_as(
        "SELECT e.payload->>'reason', (e.payload->>'actor')::uuid \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'principal_erasure_refused'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("the refusal is recorded");
    assert_eq!(reason, "unauthorized");
    assert_eq!(actor, non_admin);

    // …and NOTHING else mutated: the subject is exactly as seeded, no completion exists.
    let (handle, email, tomb): (String, Option<String>, Option<i32>) = sqlx::query_as(
        "SELECT handle, email, (tombstoned_at IS NOT NULL)::int AS tomb \
           FROM kb_profiles WHERE id = $1",
    )
    .bind(subject)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(handle, subject_handle);
    assert_eq!(email.as_deref(), Some(subject_email.as_str()));
    assert_eq!(tomb, Some(0), "a refused attempt never tombstones");
    let completions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'principal_erased'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(completions, 0, "no completion behind a refusal");

    // ── THE BITE: stand the gate down and the SAME request goes through. This is what proves
    // the 404 above was the SERVICE gate's doing and not the router's: the caller, the route,
    // the standing (approved) and the request are all unchanged — only `is_system_admin`
    // moved.
    temper_services::test_support::grant_governance(&app.pool, non_admin).await;
    let resp = app
        .client
        .post(app.url("/api/admin/erasure"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "subject": subject,
            "request_reference": request_reference,
        }))
        .send()
        .await
        .expect("the door answers again");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "with the gate stood down the same request completes: {}",
        resp.text().await.unwrap_or_default()
    );
    let body: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(body["status"], "completed", "{body}");
}

// ── WITNESS: the survey door — the operator's read-only preview ──────────────────────────────

/// FAILS IF the survey door writes anything or pretends to have an event: an operator's
/// survey answers 200 with the prediction shape (no `event_id`, no `status` tag — nothing
/// fired) and the LEDGER GAINED NOTHING — not just no erasure events, no events at all.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_operator_surveys_through_the_door_and_the_ledger_gains_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) =
        provision_and_make_operator(&app, "survey-operator", "survey-op@example.com").await;
    let (subject, _, _) = insert_profile(&app.pool).await;

    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let resp = app
        .client
        .post(app.url("/api/admin/erasure/survey"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "subject": subject }))
        .send()
        .await
        .expect("the survey door answers");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "an operator's survey answers: {}",
        resp.text().await.unwrap_or_default()
    );
    let body: Value = resp.json().await.expect("the prediction body");
    assert_eq!(body["subject"], Value::String(subject.to_string()));
    assert_eq!(body["already_erased"], false);
    assert!(
        body["targets"].is_array() && !body["targets"].as_array().unwrap().is_empty(),
        "the prediction carries the per-target outcomes: {body}"
    );
    assert!(body["redacted_hashes"].is_array());
    assert!(body["blob_strikes"].is_array());
    assert!(
        body.get("event_id").is_none(),
        "the survey fired nothing — there is no event id to report: {body}"
    );
    assert!(
        body.get("status").is_none(),
        "the survey is a plain prediction, not the tagged act outcome: {body}"
    );

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_events")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        events_before,
        "the ledger gained NOTHING from a survey"
    );
}

// ── WITNESS: the survey door's SILENT 404 — a refused survey records NOTHING ─────────────────

/// FAILS IF a non-operator's survey mutates the ledger or leaks anything but 404 — and this is
/// the witness that bites if someone "fixes" the survey to record a refusal: a survey attempt
/// is NOT an erasure request (ruled 2026-09-12), so there must be ZERO new events of any kind.
/// The bite probe stands the gate down for the SAME caller: the survey answers.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_operator_survey_gets_404_and_zero_new_events_until_the_gate_stands_down(
    pool: PgPool,
) {
    let app = common::setup_test_app(pool).await;
    let (subject, _, _) = insert_profile(&app.pool).await;
    let (token, non_admin) =
        provision_non_operator(&app, "survey-nonadmin", "survey-nonadmin@example.com").await;

    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let resp = app
        .client
        .post(app.url("/api/admin/erasure/survey"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "subject": subject }))
        .send()
        .await
        .expect("the survey door answers");
    assert_eq!(
        resp.status().as_u16(),
        404,
        "the door renders ABSENT to a caller the gate declined, got {}",
        resp.text().await.unwrap_or_default()
    );

    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        events_after, events_before,
        "a refused survey records NOTHING — no principal_erasure_refused, no event at all"
    );

    // THE GATE-BEFORE-EXISTENCE ORDER, witnessed by the 404 BODY: a non-operator surveying a
    // subject that does NOT exist still gets the gate's face — a body that says EXACTLY
    // "not found", never "profile not found". This bites if anyone moves the existence check
    // above the gate, which would leak that semantics to a caller the gate already declined.
    let ghost = Uuid::now_v7();
    let resp = app
        .client
        .post(app.url("/api/admin/erasure/survey"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "subject": ghost }))
        .send()
        .await
        .expect("the survey door answers");
    assert_eq!(
        resp.status().as_u16(),
        404,
        "the gate renders ABSENT before existence is ever consulted"
    );
    let body: Value = resp.json().await.expect("the 404 body");
    assert_eq!(
        body["error"]["message"], "not found",
        "the gate's silent face is EXACTLY \"not found\" — \"profile not found\" would \
         betray an existence check running above the gate"
    );

    // THE BITE: the same caller, the gate granted, the same call — the survey answers.
    temper_services::test_support::grant_governance(&app.pool, non_admin).await;
    let resp = app
        .client
        .post(app.url("/api/admin/erasure/survey"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "subject": subject }))
        .send()
        .await
        .expect("the survey door answers again");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "with the gate stood down the same survey answers: {}",
        resp.text().await.unwrap_or_default()
    );
}

// ── WITNESS: survey-then-execute parity through the doors ────────────────────────────────────

/// FAILS IF the survey's prediction can diverge from what the act then records: the recorded
/// `principal_erased` payload's targets must EQUAL the survey's predicted targets — exact
/// prose, exact order — and the redacted set with them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_acts_recorded_targets_equal_the_survey_s_prediction(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) =
        provision_and_make_operator(&app, "parity-operator", "parity-op@example.com").await;
    let (subject, _, _) = insert_profile(&app.pool).await;

    let surveyed = app
        .client
        .post(app.url("/api/admin/erasure/survey"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "subject": subject }))
        .send()
        .await
        .expect("the survey door answers");
    assert_eq!(surveyed.status().as_u16(), 200);
    let prediction: Value = surveyed.json().await.expect("the prediction body");

    let executed = app
        .client
        .post(app.url("/api/admin/erasure"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "subject": subject,
            "request_reference": Uuid::now_v7(),
        }))
        .send()
        .await
        .expect("the execute door answers");
    assert_eq!(
        executed.status().as_u16(),
        200,
        "the act completes: {}",
        executed.text().await.unwrap_or_default()
    );
    let (payload, _) = the_completion(&app).await;

    assert_eq!(
        payload["targets"], prediction["targets"],
        "the act's recorded targets must equal the survey's prediction, exactly"
    );
    assert_eq!(
        payload["redacted_hashes"], prediction["redacted_hashes"],
        "the redacted set must agree with the prediction"
    );
}

// ── WITNESS: the audit read is the EXISTING admin ledger surface ─────────────────────────────

/// FAILS IF the admin ledger fails to list BOTH erasure families for a system admin, or if any
/// entry re-identifies the subject: the response body is searched for the subject's HANDLE and
/// EMAIL verbatim — the record may name the subject only as the pseudonym the act broke.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_admin_ledger_lists_both_families_with_the_subject_only_as_a_pseudonym(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) =
        provision_and_make_operator(&app, "erasure-auditor", "auditor@example.com").await;
    let (subject, subject_handle, subject_email) = insert_profile(&app.pool).await;

    // One completion (the operator) and one refusal (a non-operator attempt) — both families.
    let (subject_token, _) =
        provision_non_operator(&app, "erasure-subject", "subject-attempter@example.com").await;
    let refused = app
        .client
        .post(app.url("/api/admin/erasure"))
        .header("Authorization", format!("Bearer {subject_token}"))
        .json(&serde_json::json!({ "subject": subject, "request_reference": Uuid::now_v7() }))
        .send()
        .await
        .expect("the non-operator attempt lands");
    assert_eq!(refused.status().as_u16(), 404);

    // (The auditor token is already an operator from `provision_and_make_operator`.)
    let completed = app
        .client
        .post(app.url("/api/admin/erasure"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "subject": subject, "request_reference": Uuid::now_v7() }))
        .send()
        .await
        .expect("the operator's act lands");
    assert_eq!(completed.status().as_u16(), 200);

    // THE AUDIT READ — the existing surface, no second door. The subject axis answers "what was
    // done TO this subject"; both erasure families carry the subject reference, so both arrive.
    let resp = app
        .client
        .get(app.url(&format!("/api/admin/ledger?subject=kb_profiles:{subject}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("the ledger answers");
    assert_eq!(resp.status().as_u16(), 200);
    let page: Value = resp.json().await.expect("the ledger page");
    let types: Vec<&str> = page["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| e["event_type"].as_str().expect("event_type"))
        .collect();
    assert!(
        types.contains(&"principal_erased"),
        "the ledger lists erasures, got {types:?}"
    );
    assert!(
        types.contains(&"principal_erasure_refused"),
        "the ledger lists refusals, got {types:?}"
    );

    // THE PSEUDONYM CEILING: the wire body must not carry the subject's identity anywhere —
    // not in the payloads, not in an actor_handle, not in a reference.
    let body = page.to_string();
    assert!(
        !body.contains(&subject_handle),
        "the subject's HANDLE must not appear in the admin ledger wire shape"
    );
    assert!(
        !body.contains(&subject_email),
        "the subject's EMAIL must not appear in the admin ledger wire shape"
    );
    // The pseudonym itself IS the record's name for the subject — that is the design.
    assert!(
        body.contains(&subject.to_string()),
        "the subject_id (the pseudonym) is the only name the record carries"
    );
}
