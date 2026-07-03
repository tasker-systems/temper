#![cfg(feature = "test-db")]
//! HTTP-layer integration tests for the internal SAML reconcile endpoint.
//! The endpoint is gated by an HMAC signature over the body (not JWT). We build the app with a
//! known `internal_reconcile_secret` and `auth_provider_name = "saml:acme"` so the JIT'd profile's
//! auth link matches what the minted token would later resolve to. Requests are signed exactly as
//! the TS Authorization Server signs them (`temper_core::internal_sig`).

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::RequestBuilder;
use sqlx::PgPool;
use temper_core::internal_sig::{sign, SIGNATURE_HEADER, TIMESTAMP_HEADER};
use temper_core::types::ReconcileRequest;
use uuid::Uuid;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Build a POST carrying a body string signed at `timestamp` with `secret`. The signed bytes
/// are exactly the bytes sent, matching the raw-body discipline the AS uses.
fn signed_post(
    builder: RequestBuilder,
    secret: &str,
    timestamp: i64,
    body: &str,
) -> RequestBuilder {
    let signature = sign(secret.as_bytes(), timestamp, body.as_bytes());
    builder
        .header("content-type", "application/json")
        .header(TIMESTAMP_HEADER, timestamp.to_string())
        .header(SIGNATURE_HEADER, signature)
        .body(body.to_string())
}

/// Seed an active IdP 'acme', a team, and a mapping engineering -> team (member). Returns team_id.
async fn seed(pool: &PgPool) -> Uuid {
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("eng-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("seed team");

    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr)
         VALUES ('acme', true, 'x', 'https://idp/sso', 'idp', 'sp', 'https://sp/acs', 'persistent', 'email', 'uid', 'groups')",
    )
    .execute(pool)
    .await
    .expect("seed idp");

    sqlx::query("INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES ('acme', 'engineering', $1, 'member')")
        .bind(team_id)
        .execute(pool)
        .await
        .expect("seed mapping");

    team_id
}

fn reconcile_body() -> ReconcileRequest {
    ReconcileRequest {
        provider: Some("saml:acme".to_string()),
        external_user_id: "nid-1".to_string(),
        email: "a@corp.io".to_string(),
        email_verified: Some(true),
        idp_key: "acme".to_string(),
        groups: vec!["engineering".to_string()],
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_provisions_idp_membership(pool: PgPool) {
    let team_id = seed(&pool).await;
    let app = common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await;

    let body = serde_json::to_string(&reconcile_body()).unwrap();
    let resp = signed_post(
        app.client.post(app.url("/internal/saml/reconcile")),
        "s3cr3t",
        now_secs(),
        &body,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        204,
        "valid signature should return 204; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // The profile was JIT-created with provider 'saml:acme', external id 'nid-1'.
    let profile_id: Uuid = sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider = $1 AND auth_provider_user_id = $2",
    )
    .bind("saml:acme")
    .bind("nid-1")
    .fetch_one(&pool)
    .await
    .expect("JIT auth link must exist");

    let (role, source): (String, String) = sqlx::query_as(
        "SELECT role::text, source::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(team_id)
    .bind(profile_id)
    .fetch_one(&pool)
    .await
    .expect("idp membership must exist");
    assert_eq!(role, "member");
    assert_eq!(source, "idp");
}

async fn assert_rejected_and_no_provisioning(pool: &PgPool, builder: RequestBuilder) {
    let resp = builder.send().await.expect("request failed");
    assert_eq!(resp.status().as_u16(), 401);

    // No profile/link/membership was created.
    let links: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider_user_id = 'nid-1'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(links, 0);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_rejects_wrong_secret(pool: PgPool) {
    seed(&pool).await;
    let app = common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await;

    // Signed with the wrong secret — a valid-looking signature that won't verify.
    let body = serde_json::to_string(&reconcile_body()).unwrap();
    let builder = signed_post(
        app.client.post(app.url("/internal/saml/reconcile")),
        "wrong-secret",
        now_secs(),
        &body,
    );
    assert_rejected_and_no_provisioning(&pool, builder).await;
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_rejects_stale_timestamp(pool: PgPool) {
    seed(&pool).await;
    let app = common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await;

    // Correctly signed, but the timestamp is an hour old — well past the freshness window.
    let body = serde_json::to_string(&reconcile_body()).unwrap();
    let builder = signed_post(
        app.client.post(app.url("/internal/saml/reconcile")),
        "s3cr3t",
        now_secs() - 3600,
        &body,
    );
    assert_rejected_and_no_provisioning(&pool, builder).await;
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_rejects_tampered_body(pool: PgPool) {
    seed(&pool).await;
    let app = common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await;

    // Sign the honest body, then send a different body under that signature.
    let signed_body = serde_json::to_string(&reconcile_body()).unwrap();
    let timestamp = now_secs();
    let signature = sign("s3cr3t".as_bytes(), timestamp, signed_body.as_bytes());
    let tampered = serde_json::to_string(&ReconcileRequest {
        groups: vec!["temper-admins".to_string()],
        ..reconcile_body()
    })
    .unwrap();
    let builder = app
        .client
        .post(app.url("/internal/saml/reconcile"))
        .header("content-type", "application/json")
        .header(TIMESTAMP_HEADER, timestamp.to_string())
        .header(SIGNATURE_HEADER, signature)
        .body(tampered);
    assert_rejected_and_no_provisioning(&pool, builder).await;
}
