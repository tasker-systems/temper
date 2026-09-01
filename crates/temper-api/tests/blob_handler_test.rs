#![cfg(feature = "test-db")]
//! HTTP-layer witnesses for the blob surfaces (segment S2 of the surfaces task, spec: binary
//! blobs 2026-09-01). Full stack via the `TestApp` harness — the threshold refusals only
//! exist at the HTTP layer, and the visibility refusal only means what it means when it
//! renders through the real router.
//!
//! The provider under test is the substrate's own `InMemoryBlobStore` — the same contract
//! `VercelBlobStore` satisfies, injected through `setup_test_app_with_state` (the seam
//! `AppState::new` builds the real client from config through). No witness here exercises the
//! live provider; that posture is declared in the goal register (deploy-time-exercised only).
//!
//! The config each app runs is the REAL `BlobConfig` vocabulary (D9: enforcement and refusal
//! come from the same values the operator set) — only the numbers are test-sized.

mod common;

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use common::fixtures;
use common::{generate_test_jwt, setup_test_app, setup_test_app_with_state, TestApp};
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_substrate::blob_store::InMemoryBlobStore;

// ─── Fixture helpers ─────────────────────────────────────────────────────────

/// A test-sized `BlobConfig`: static-token posture (no env races), the given D9 numbers.
fn blob_cfg(max_bytes: i64, allowlist: &[&str], threshold: usize) -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes,
        allowlist: allowlist.iter().map(|s| s.to_string()).collect(),
        single_request_max_bytes: threshold,
    }
}

/// Build the app with a fake provider and the given real-config vocabulary.
async fn blob_app(pool: PgPool, cfg: BlobConfig) -> TestApp {
    setup_test_app_with_state(pool, move |state| {
        state.blob_store = Some(Arc::new(InMemoryBlobStore::default()));
        let mut config = (*state.config).clone();
        config.blob = Some(cfg);
        state.config = Arc::new(config);
    })
    .await
}

/// A fully-provisioned profile with its own `temper` context, plus the JWT that reaches it.
async fn owner(pool: &PgPool) -> (Uuid, Uuid, String) {
    let email = format!("blob-owner-{}@example.com", Uuid::new_v4());
    let (profile, ctx) = fixtures::create_test_profile_with_context(pool, &email).await;
    let token = generate_test_jwt(&format!("test|{profile}"), &email);
    (profile, ctx, token)
}

fn commit_multipart(
    app: &TestApp,
    token: &str,
    bytes: Vec<u8>,
    mime: &str,
    home_table: &str,
    home_id: Uuid,
) -> reqwest::RequestBuilder {
    let file = reqwest::multipart::Part::bytes(bytes)
        .file_name("figure.png")
        .mime_str(mime)
        .expect("mime");
    let form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("home_table", home_table.to_string())
        .text("home_id", home_id.to_string());
    app.client
        .post(app.url("/api/blobs"))
        .header("Authorization", format!("Bearer {token}"))
        .multipart(form)
}

// ─── Witness 1: blob-bytes-retrievable-whole, at the surface layer ───────────

/// What was committed is what comes back: same bytes, byte for byte; the response speaks the
/// STORED content type; `Cache-Control: immutable` rides the read (D6 — content addressing
/// earns the strongest cache posture); and `Content-Length` is the committed count.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn committed_bytes_come_back_whole(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png", "application/pdf"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let bytes: Vec<u8> = (0..12_345u32).map(|i| (i * 7 % 251) as u8).collect();
    let resp = commit_multipart(&app, &token, bytes.clone(), "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(status, 200, "commit should succeed; body: {body}");
    assert_eq!(body["deduped"], false, "fresh bytes are not a dedup hit");
    let blob_id = body["blob_id"].as_str().expect("blob_id").to_string();

    let resp = app
        .client
        .get(app.url(&format!("/api/blobs/{blob_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "read must succeed");
    assert_eq!(
        resp.headers()["content-type"],
        "image/png",
        "the response speaks the stored media type, not a generic one"
    );
    let cache_control = resp
        .headers()
        .get("cache-control")
        .expect("cache-control header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_control.contains("immutable"),
        "content-addressed bytes are immutable; cache-control was {cache_control}"
    );
    let back = resp.bytes().await.expect("body");
    assert_eq!(
        back.as_ref(),
        bytes.as_slice(),
        "what was committed is what was retrieved, whole"
    );
}

// ─── Witness 2: blob-visibility-self-contained at the read gate ──────────────

/// A blob homed in someone else's context does not exist for an outsider: 404, and the SAME
/// 404 an unknown id gets — a probe cannot tell an invisible blob from an absent one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_invisible_blob_is_not_found(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_owner_profile, ctx, owner_token) = owner(&app.pool).await;

    let outsider_email = format!("blob-out-{}@example.com", Uuid::new_v4());
    let (outsider, _) =
        fixtures::create_test_profile_with_context(&app.pool, &outsider_email).await;
    let outsider_token = generate_test_jwt(&format!("test|{outsider}"), &outsider_email);

    let bytes: Vec<u8> = vec![1, 2, 3, 4, 5];
    let resp = commit_multipart(&app, &owner_token, bytes, "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let committed: serde_json::Value = resp.json().await.expect("json");
    let blob_id = committed["blob_id"].as_str().unwrap().to_string();

    let outsider_view = app
        .client
        .get(app.url(&format!("/api/blobs/{blob_id}")))
        .header("Authorization", format!("Bearer {outsider_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(outsider_view.status().as_u16(), 404, "invisible = absent");

    let unknown_id = Uuid::now_v7();
    let probe = app
        .client
        .get(app.url(&format!("/api/blobs/{unknown_id}")))
        .header("Authorization", format!("Bearer {outsider_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(probe.status().as_u16(), 404, "unknown = absent too");

    let probe_text = probe.text().await.unwrap_or_default();
    let outsider_text = outsider_view.text().await.unwrap_or_default();
    assert_eq!(
        probe_text, outsider_text,
        "an invisible blob and an unknown one render the SAME refusal"
    );
}

// ─── Witness 3: refusal-names-its-vocabulary, with real config ───────────────

/// The threshold refusal names the threshold in force and the segmented path beyond it; the
/// allowlist refusal (the SQL wrapper's, surfaced verbatim) names the allowlist in force; the
/// home refusal names the two anchors a blob can live in; an unconfigured instance refuses
/// with what would enable it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn refusals_name_their_vocabulary(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 1024);
    let app = blob_app(pool.clone(), cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    // Over the single-request threshold — refused DURING the read, naming threshold + cap +
    // the segmented path.
    let over: Vec<u8> = vec![0u8; 2048];
    let resp = commit_multipart(&app, &token, over, "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("single-request threshold") && text.contains("1024"),
        "refusal must name the threshold in force: {text}"
    );
    assert!(
        text.contains("segmented"),
        "refusal must name the path beyond the threshold: {text}"
    );

    // Off the allowlist — the SQL wrapper's refusal, verbatim, naming the allowlist in force.
    let resp = commit_multipart(
        &app,
        &token,
        b"plain text".to_vec(),
        "text/plain",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("not admitted") && text.contains("image/png"),
        "refusal must name the allowlist in force: {text}"
    );

    // Unknown home anchor — refused in the wrapper's terms.
    let resp = commit_multipart(
        &app,
        &token,
        b"x".to_vec(),
        "image/png",
        "kb_teams",
        Uuid::now_v7(),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("kb_contexts") && text.contains("kb_cogmaps"),
        "refusal must name the home vocabulary: {text}"
    );

    // Unconfigured instance — absent, not broken, and the refusal says what enables the door.
    let plain = setup_test_app(pool.clone()).await;
    let (_p2, ctx2, token2) = owner(&plain.pool).await;
    let resp = commit_multipart(
        &plain,
        &token2,
        b"x".to_vec(),
        "image/png",
        "kb_contexts",
        ctx2,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("disabled") && text.contains("BLOB_STORE_ID"),
        "the disabled refusal must name its config vocabulary: {text}"
    );
}

// ─── Witness 4: dedup for free, first home stands (D1/D2) ────────────────────

/// The same bytes committed twice are ONE row: the second commit reports deduped=true with the
/// existing id, and no second row exists behind the hash.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_same_bytes_committed_twice_are_one_blob(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let bytes: Vec<u8> = b"deterministic bytes".to_vec();
    let first = commit_multipart(&app, &token, bytes.clone(), "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(first.status().as_u16(), 200);
    let first: serde_json::Value = first.json().await.expect("json");

    let second = commit_multipart(&app, &token, bytes, "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(second.status().as_u16(), 200);
    let second: serde_json::Value = second.json().await.expect("json");

    assert_eq!(second["deduped"], true, "the second commit is a dedup hit");
    assert_eq!(
        first["blob_id"], second["blob_id"],
        "dedup returns the existing blob — the first home stands"
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_blobs WHERE content_hash = $1")
        .bind(first["content_hash"].as_str().unwrap())
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(rows, 1, "one hash, one row");
}

// ─── Witness 5: the cap is the wrapper's, taught from real config ────────────

/// Under the threshold but over the cap: the wrapper refuses, and the refusal names the cap in
/// force — the same config value the operator set, not a code constant.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn over_the_cap_refuses_naming_the_cap(pool: PgPool) {
    // Cap BELOW the threshold: the threshold gate passes, the wrapper's must fire.
    let cfg = blob_cfg(10, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let resp = commit_multipart(&app, &token, vec![9u8; 20], "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("exceeds the configured per-blob cap") && text.contains("10"),
        "refusal must name the cap in force (10): {text}"
    );
}

// ─── Witness 6: home standing, the placement gate ────────────────────────────

/// A caller who cannot READ the home gets 404 (absent, not refused); a caller who can read but
/// not write gets 403 — the two-step placement gate (`anchor_readable_by_profile`, then
/// `context_authorable_by_profile`), the same posture resource placement has had since F-2.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_home_gate_refuses_under_scoped_writes(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_owner_profile, ctx, owner_token) = owner(&app.pool).await;

    // Invisible home → 404.
    let out_email = format!("blob-gate-out-{}@example.com", Uuid::new_v4());
    let (outsider, _) = fixtures::create_test_profile_with_context(&app.pool, &out_email).await;
    let outsider_token = generate_test_jwt(&format!("test|{outsider}"), &out_email);
    let resp = commit_multipart(
        &app,
        &outsider_token,
        b"x".to_vec(),
        "image/png",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 404, "an unreadable home is absent");

    // Readable-but-not-writable home → 403: an explicit context read grant, no write.
    let reader_email = format!("blob-gate-ro-{}@example.com", Uuid::new_v4());
    let (reader, _) = fixtures::create_test_profile_with_context(&app.pool, &reader_email).await;
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, false, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(ctx)
    .bind(reader)
    .execute(&app.pool)
    .await
    .expect("grant read");
    let reader_token = generate_test_jwt(&format!("test|{reader}"), &reader_email);

    let resp = commit_multipart(
        &app,
        &reader_token,
        b"x".to_vec(),
        "image/png",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "read is strictly broader than write: a reader cannot place a blob"
    );

    // The owner, who both reads and authors the context, sails through.
    let resp = commit_multipart(
        &app,
        &owner_token,
        b"x".to_vec(),
        "image/png",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "owner: body {resp:?}");
}
