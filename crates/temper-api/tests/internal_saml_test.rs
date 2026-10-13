#![cfg(feature = "test-db")]
//! HTTP-layer integration tests for the internal SAML reconcile endpoint.
//! The endpoint is gated by an HMAC signature over the body (not JWT). We build the app with a
//! known `internal_reconcile_secret` and `auth_provider_name = "saml:acme"` so the JIT'd profile's
//! auth link matches what the minted token would later resolve to. Requests are signed exactly as
//! the TS Authorization Server signs them (`temper_core::internal_sig`).

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::RequestBuilder;
use sha2::Sha256;
use sqlx::PgPool;
use temper_core::internal_sig::{
    v2_headers, DOORS, DOOR_PRINCIPAL_RESOLVE, DOOR_SAML_RECONCILE, TIMESTAMP_HEADER,
};
use temper_core::types::ReconcileRequest;
use uuid::Uuid;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The retired `"{timestamp}.{body}"` construction (`X-Temper-Signature`), built here because
/// nothing in `src/` signs it any more. It exists only so a test can show it is refused.
fn sign_v1(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// A fresh v2 nonce, as the AS's `randomBytes(16)` would make one.
fn fresh_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Add v2 headers signing `body` for `door` at `timestamp` under `nonce`.
fn with_v2(
    mut builder: RequestBuilder,
    secret: &str,
    door: &str,
    timestamp: i64,
    nonce: &str,
    body: &str,
) -> RequestBuilder {
    for (name, value) in v2_headers(
        secret.as_bytes(),
        "POST",
        door,
        timestamp,
        nonce,
        body.as_bytes(),
    ) {
        builder = builder.header(name, value);
    }
    builder
}

/// Build a POST to the reconcile door carrying a body string signed at `timestamp` with `secret`,
/// under a fresh nonce. The signed bytes are exactly the bytes sent, matching the raw-body
/// discipline the AS uses.
fn signed_post(
    builder: RequestBuilder,
    secret: &str,
    timestamp: i64,
    body: &str,
) -> RequestBuilder {
    with_v2(
        builder,
        secret,
        DOOR_SAML_RECONCILE,
        timestamp,
        &fresh_nonce(),
        body,
    )
    .header("content-type", "application/json")
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
        groups: Some(vec!["engineering".to_string()]),
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
    let tampered = serde_json::to_string(&ReconcileRequest {
        groups: Some(vec!["temper-admins".to_string()]),
        ..reconcile_body()
    })
    .unwrap();
    let builder = with_v2(
        app.client.post(app.url("/internal/saml/reconcile")),
        "s3cr3t",
        DOOR_SAML_RECONCILE,
        timestamp,
        &fresh_nonce(),
        &signed_body,
    )
    .header("content-type", "application/json")
    .body(tampered);
    assert_rejected_and_no_provisioning(&pool, builder).await;
}

/// A `groups: null` payload takes the no-signal path end to end — over HTTP, through serde, into
/// the record.
///
/// The service-level witnesses in `temper-services` prove the guard refuses to revoke. This one
/// proves the distinction SURVIVES THE WIRE, which is the half they cannot see: `null` and `[]`
/// are one JSON character apart and land in the same field, and a `Vec<String>` on this side would
/// have deserialized both to the same value with no error anywhere. The endpoint still answers 204
/// — declining to act on a signal that was not sent is a well-formed outcome, not a failure, and
/// the AS treats a non-2xx as a reconcile failure.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_records_a_null_group_signal_as_a_skip(pool: PgPool) {
    seed(&pool).await;
    let app = common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await;

    let body = serde_json::to_string(&ReconcileRequest {
        groups: None,
        ..reconcile_body()
    })
    .unwrap();
    assert!(
        body.contains(r#""groups":null"#),
        "the payload must carry an explicit null, not an omitted key: {body}"
    );

    let resp = signed_post(
        app.client.post(app.url("/internal/saml/reconcile")),
        "s3cr3t",
        now_secs(),
        &body,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 204);

    let profile_id: Uuid = sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider = $1 AND auth_provider_user_id = $2",
    )
    .bind("saml:acme")
    .bind("nid-1")
    .fetch_one(&pool)
    .await
    .expect("JIT auth link must exist");

    let (reconciled, skipped): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT last_reconciled_at, last_skipped_at FROM kb_saml_principal_reconcile
          WHERE profile_id = $1 AND idp_key = 'acme'",
    )
    .bind(profile_id)
    .fetch_one(&pool)
    .await
    .expect("the declined attempt is recorded");

    assert!(skipped.is_some(), "the skip is timestamped");
    assert!(
        reconciled.is_none(),
        "and reads as no agreement, not a fresh one"
    );

    // Scoped to source='idp' deliberately: resolving the principal JITs a profile, and profile
    // provisioning gives it a native membership of its own. That row is not what the guard is
    // about, and counting it would make this assertion fail for a reason unrelated to the claim.
    let idp_memberships: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members WHERE profile_id = $1 AND source = 'idp'",
    )
    .bind(profile_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        idp_memberships, 0,
        "and no idp-derived reach was conferred from no signal"
    );
}

/// The configuration every single-use and door test below runs under.
async fn reconcile_app(pool: &PgPool) -> common::TestApp {
    common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await
}

/// A signed request is good once. The same request sent again inside its freshness window is
/// refused: before v2, it verified again.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_refuses_a_replay(pool: PgPool) {
    seed(&pool).await;
    let app = reconcile_app(&pool).await;
    let body = serde_json::to_string(&reconcile_body()).unwrap();
    let (timestamp, nonce) = (now_secs(), fresh_nonce());
    let send = || {
        with_v2(
            app.client.post(app.url("/internal/saml/reconcile")),
            "s3cr3t",
            DOOR_SAML_RECONCILE,
            timestamp,
            &nonce,
            &body,
        )
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
    };

    let first = send().await.expect("first send");
    assert!(
        first.status().is_success(),
        "the first send is accepted: {}",
        first.status()
    );
    let replay = send().await.expect("replay");
    assert_eq!(
        replay.status(),
        401,
        "the same request again must be refused"
    );
}

/// A signature is good for the door it names. A body signed for reconcile and sent to resolve —
/// the sibling door on the same key — is refused.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolve_refuses_a_body_signed_for_reconcile(pool: PgPool) {
    let app = reconcile_app(&pool).await;
    let body = r#"{"provider":"saml:acme","external_user_id":"nid-1"}"#;
    let resp = with_v2(
        app.client.post(app.url(DOOR_PRINCIPAL_RESOLVE)),
        "s3cr3t",
        DOOR_SAML_RECONCILE,
        now_secs(),
        &fresh_nonce(),
        body,
    )
    .header("content-type", "application/json")
    .body(body)
    .send()
    .await
    .expect("send");
    assert_eq!(
        resp.status(),
        401,
        "a signature for another door must be refused"
    );
}

/// The original construction alone is no longer accepted, however fresh: accepting it would leave
/// its replay and cross-door gaps open to anyone who strips the v2 headers from a capture.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_refuses_the_original_construction(pool: PgPool) {
    seed(&pool).await;
    let app = reconcile_app(&pool).await;
    let body = serde_json::to_string(&reconcile_body()).unwrap();
    let timestamp = now_secs();
    let builder = app
        .client
        .post(app.url("/internal/saml/reconcile"))
        .header("content-type", "application/json")
        .header(TIMESTAMP_HEADER, timestamp.to_string())
        .header(
            "X-Temper-Signature",
            sign_v1("s3cr3t".as_bytes(), timestamp, body.as_bytes()),
        )
        .body(body);
    assert_rejected_and_no_provisioning(&pool, builder).await;
}

/// Every door the signers name is a route the API serves, so a signer and the route cannot drift
/// apart unnoticed: an unsigned POST to each is refused by its gate (401), never unrouted (404).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_signed_door_is_a_gated_route(pool: PgPool) {
    let app = reconcile_app(&pool).await;
    for door in DOORS {
        let resp = app
            .client
            .post(app.url(door))
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status(), 401, "{door} must be a gated route");
    }
}
