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
/// STORED content type; `Cache-Control` is `private, immutable` (D6 — content addressing earns
/// `immutable`; the bytes are per-caller authorized, so a shared cache is never licensed to
/// store them); and `Content-Length` is the committed count.
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
    // FAILS IF: the read-through ever licenses a SHARED cache to store per-caller-authorized
    // bytes — `public` is exactly that license (RFC 7234 §3.2), and no `private` means an
    // Authorization-bearing response is storable and servable to any other principal.
    assert!(
        !cache_control.contains("public"),
        "an authorized read-through must never license shared caching; cache-control was \
         {cache_control}"
    );
    assert!(
        cache_control.contains("private"),
        "per-caller-authorized bytes are privately cacheable at most; cache-control was \
         {cache_control}"
    );
    // FAILS IF: the read-through ever renders instead of downloading. `attachment` is the
    // F10 ruling — the posture that survives a cookie-auth flip, a CSP relaxation, or an
    // allowlist edit (commit-time only, so committed SVG stays served): navigation
    // downloads instead of executing in the app's origin, while `<img>`/`<video>`
    // subresource rendering (which ignores this header) is unaffected.
    assert_eq!(
        resp.headers()["content-disposition"],
        temper_services::services::blob_service::BLOB_CONTENT_DISPOSITION,
        "a blob read is a bytes fetch, never a rendering invitation"
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

    // Deliberately closed — `BLOB_ENABLED=false` is a DECISION, and the refusal must
    // name the knob, never the credential vocabulary (the unconfigured sentence above
    // would invite an operator to set what they deliberately withheld).
    let policy = setup_test_app_with_state(pool.clone(), |state| {
        let mut config = (*state.config).clone();
        config.blob_disabled_by_policy = true;
        state.config = Arc::new(config);
    })
    .await;
    let (_p3, ctx3, token3) = owner(&policy.pool).await;
    let resp = commit_multipart(
        &policy,
        &token3,
        b"x".to_vec(),
        "image/png",
        "kb_contexts",
        ctx3,
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("disabled by configuration") && text.contains("BLOB_ENABLED"),
        "the policy refusal must name the opt-out knob: {text}"
    );
    assert!(
        !text.contains("no blob store configured"),
        "the policy refusal must not borrow the unconfigured vocabulary: {text}"
    );
}

// ─── Witness 4: dedup for free, per-home get-or-create (D1/D2 as amended) ─────

/// The same bytes committed twice to the SAME home are ONE row: the second commit reports
/// deduped=true with the same id, and no second row exists behind the hash.
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
        "dedup within a home returns the same blob (per-home get-or-create)"
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_blobs WHERE content_hash = $1")
        .bind(first["content_hash"].as_str().unwrap())
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(rows, 1, "one hash, one row");
}

// ─── Witness 4b: a dedup hit reports the STORED content type (N2) ─────────────

/// A re-commit that DECLARES a different media type gets the row's STORED type back — the
/// first committer's, which is what read-through serves. FAILS IF: the response echoes the
/// caller's declaration (content-type drift: the ledger says one type, the bytes serve
/// another, and the caller's client records a type that was never stored).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_dedup_hit_reports_the_stored_content_type(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png", "text/plain"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let bytes: Vec<u8> = b"content-type-drift-witness".to_vec();
    let first = commit_multipart(&app, &token, bytes.clone(), "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(first.status().as_u16(), 200);
    let first: serde_json::Value = first.json().await.expect("json");
    assert_eq!(
        first["content_type"], "image/png",
        "a fresh commit stores its declaration"
    );

    // Same bytes, same home, DIFFERENT declared type: the row keeps the first committer's
    // type (the projector's conflict arm never updates it), so the response must report
    // what is STORED, not what this request declared.
    let second = commit_multipart(&app, &token, bytes, "text/plain", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(second.status().as_u16(), 200);
    let second: serde_json::Value = second.json().await.expect("json");
    assert_eq!(second["deduped"], true, "the re-commit is a dedup hit");
    assert_eq!(
        second["content_type"], "image/png",
        "a dedup hit reports the row's stored media type, never the re-commit's declaration"
    );
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

// ─── S3 witnesses: the segmented upload path ─────────────────────────────────

fn sha_hex(bytes: &[u8]) -> String {
    temper_core::hash::sha256_hex(bytes)
}

async fn begin_upload(
    app: &TestApp,
    token: &str,
    home_table: &str,
    home_id: Uuid,
) -> reqwest::Response {
    app.client
        .post(app.url("/api/blobs/uploads"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "home_table": home_table,
            "home_id": home_id,
            "content_type": "image/png",
        }))
        .send()
        .await
        .expect("request failed")
}

fn append_segment_req(
    app: &TestApp,
    token: &str,
    upload_id: Uuid,
    seq: u32,
    bytes: &[u8],
) -> reqwest::RequestBuilder {
    // No integrity header: the segment's identity is the SERVER's own sha256 of the
    // bytes received (F7 ruling — the client-sent `x-segment-sha256` was validated then
    // discarded, a dead check inviting a future caller to consume it unverified).
    app.client
        .post(app.url(&format!(
            "/api/blobs/uploads/{upload_id}/segments?seq={seq}"
        )))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(bytes.to_vec())
}

async fn finalize_upload(
    app: &TestApp,
    token: &str,
    upload_id: Uuid,
    expected_segments: u32,
    expected_total_bytes: i64,
    expected_content_hash: Option<String>,
) -> reqwest::Response {
    let mut payload = serde_json::json!({
        "expected_segments": expected_segments,
        "expected_total_bytes": expected_total_bytes,
    });
    if let Some(hash) = expected_content_hash {
        payload["expected_content_hash"] = serde_json::Value::String(hash);
    }
    app.client
        .post(app.url(&format!("/api/blobs/uploads/{upload_id}/finalize")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .send()
        .await
        .expect("request failed")
}

/// The whole path, end to end: staged segments commit bytes WHOLE (identical hash and bytes
/// to what a single-request commit of the same file produces), the media type declared at
/// begin is the one the read-back speaks, and the same bytes committed again through the
/// OTHER path are a dedup hit on the same id — one content-addressed blob, two upload paths.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn segmented_upload_commits_bytes_whole_and_dedups_across_paths(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let seg_a: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let seg_b: Vec<u8> = (0..10_480u32).map(|i| (i * 13 % 241) as u8).collect();
    let whole = [seg_a.clone(), seg_b.clone()].concat();

    let resp = begin_upload(&app, &token, "kb_contexts", ctx).await;
    assert_eq!(resp.status().as_u16(), 200, "begin must succeed");
    let upload_id: Uuid = resp.json::<serde_json::Value>().await.expect("json")["upload_id"]
        .as_str()
        .expect("upload_id")
        .parse()
        .expect("uuid");

    let resp = append_segment_req(&app, &token, upload_id, 0, &seg_a)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let progress: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(progress["segments"].as_array().unwrap().len(), 1);
    assert_eq!(progress["total_bytes"], seg_a.len() as i64);
    assert_eq!(
        progress["segments"][0]["segment_hash"],
        sha_hex(&seg_a),
        "the progress read reports the landed segment's hash"
    );

    let resp = append_segment_req(&app, &token, upload_id, 1, &seg_b)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    // The idempotent re-send: same segment, same seq — a no-op, same progress.
    let resp = append_segment_req(&app, &token, upload_id, 1, &seg_b)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "idempotent re-send succeeds");
    let progress: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(
        progress["segments"].as_array().unwrap().len(),
        2,
        "the re-send landed nothing new"
    );
    assert_eq!(progress["total_bytes"], (seg_a.len() + seg_b.len()) as i64);

    let resp = finalize_upload(
        &app,
        &token,
        upload_id,
        2,
        (seg_a.len() + seg_b.len()) as i64,
        Some(sha_hex(&whole)),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "finalize must succeed");
    let committed: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(
        committed["deduped"], false,
        "fresh bytes are not a dedup hit"
    );
    assert_eq!(
        committed["content_hash"],
        sha_hex(&whole),
        "the committed hash is the assembled whole's"
    );
    assert_eq!(
        committed["content_type"], "image/png",
        "the type declared at begin is stored"
    );
    assert_eq!(committed["content_bytes"], whole.len() as i64);
    let blob_id = committed["blob_id"].as_str().unwrap().to_string();

    let resp = app
        .client
        .get(app.url(&format!("/api/blobs/{blob_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let back = resp.bytes().await.expect("body");
    assert_eq!(
        back.as_ref(),
        whole.as_slice(),
        "staged in two segments, read back whole — byte for byte"
    );

    // The single-request path, same bytes: a dedup hit on the SAME id.
    let resp = commit_multipart(&app, &token, whole, "image/png", "kb_contexts", ctx)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let again: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(
        again["deduped"], true,
        "the second path's commit is a dedup hit"
    );
    assert_eq!(
        again["blob_id"], committed["blob_id"],
        "one blob, two upload paths"
    );

    // Success deleted the staging: the session is gone for its owner too.
    let resp = app
        .client
        .get(app.url(&format!("/api/blobs/uploads/{upload_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        404,
        "a finalized session no longer exists"
    );
}

/// The staging ceiling (`max_bytes`, the cumulative bound across appends) refuses with its
/// own vocabulary — `blob_upload:`, naming the ceiling in force — and the refused append
/// changed nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_staging_ceiling_refuses_and_names_itself(pool: PgPool) {
    let cfg = blob_cfg(1024, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let resp = begin_upload(&app, &token, "kb_contexts", ctx).await;
    assert_eq!(resp.status().as_u16(), 200);
    let upload_id: Uuid = resp.json::<serde_json::Value>().await.expect("json")["upload_id"]
        .as_str()
        .expect("upload_id")
        .parse()
        .expect("uuid");

    let first: Vec<u8> = vec![7u8; 600];
    let resp = append_segment_req(&app, &token, upload_id, 0, &first)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "under the ceiling lands");

    let second: Vec<u8> = vec![8u8; 600];
    let resp = append_segment_req(&app, &token, upload_id, 1, &second)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400, "over the ceiling refuses");
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("blob_upload:") && text.contains("1024"),
        "the refusal must name the staging vocabulary and the ceiling in force: {text}"
    );

    let resp = app
        .client
        .get(app.url(&format!("/api/blobs/uploads/{upload_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    let progress: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(
        progress["total_bytes"], 600i64,
        "the refused append changed nothing"
    );
}

/// The finalize refusals keep the staging: stale concurrency tokens are 409 (resumable —
/// re-read the progress, re-finalize), an integrity mismatch is 422 (the ingest precedent's
/// face), and after both the staging is intact and the finalize with the right tokens
/// succeeds — a refusal never commits, and a success deletes the staging.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn finalize_failures_keep_the_staging_resumable(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let seg_a: Vec<u8> = vec![1u8; 512];
    let seg_b: Vec<u8> = vec![2u8; 512];
    let whole = [seg_a.clone(), seg_b.clone()].concat();

    let resp = begin_upload(&app, &token, "kb_contexts", ctx).await;
    let upload_id: Uuid = resp.json::<serde_json::Value>().await.expect("json")["upload_id"]
        .as_str()
        .expect("upload_id")
        .parse()
        .expect("uuid");
    append_segment_req(&app, &token, upload_id, 0, &seg_a)
        .send()
        .await
        .expect("request failed");
    append_segment_req(&app, &token, upload_id, 1, &seg_b)
        .send()
        .await
        .expect("request failed");

    let resp = finalize_upload(&app, &token, upload_id, 3, 1024, None).await;
    assert_eq!(resp.status().as_u16(), 409, "stale segment count refuses");
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("2") && text.contains("3"),
        "the refusal names both counts: {text}"
    );

    let resp = finalize_upload(&app, &token, upload_id, 2, 999, None).await;
    assert_eq!(resp.status().as_u16(), 409, "stale byte total refuses");

    let resp = finalize_upload(
        &app,
        &token,
        upload_id,
        2,
        1024,
        Some(sha_hex(b"not-the-bytes")),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 422, "an integrity mismatch is 422");
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("never superseded"),
        "the integrity refusal says the staging cannot be patched in place: {text}"
    );

    // Staging intact through every refusal.
    let resp = app
        .client
        .get(app.url(&format!("/api/blobs/uploads/{upload_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "the staging survives every refusal"
    );
    let progress: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(progress["segments"].as_array().unwrap().len(), 2);
    assert_eq!(progress["total_bytes"], 1024i64);

    // The correct tokens and hash commit.
    let resp = finalize_upload(&app, &token, upload_id, 2, 1024, Some(sha_hex(&whole))).await;
    assert_eq!(resp.status().as_u16(), 200, "the correct finalize commits");
}

/// A staged session is caller-private at the HTTP layer: append, progress, and finalize all
/// answer another profile with the SAME 404 an unknown id gets — a probe over upload ids
/// learns nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_staged_session_is_caller_private(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;
    let out_email = format!("staging-out-{}@example.com", Uuid::new_v4());
    let (outsider, _) = fixtures::create_test_profile_with_context(&app.pool, &out_email).await;
    let outsider_token = generate_test_jwt(&format!("test|{outsider}"), &out_email);

    let resp = begin_upload(&app, &token, "kb_contexts", ctx).await;
    assert_eq!(resp.status().as_u16(), 200);
    let upload_id: Uuid = resp.json::<serde_json::Value>().await.expect("json")["upload_id"]
        .as_str()
        .expect("upload_id")
        .parse()
        .expect("uuid");

    for (label, resp) in [
        (
            "append",
            append_segment_req(&app, &outsider_token, upload_id, 0, b"x")
                .send()
                .await
                .expect("request failed"),
        ),
        (
            "progress",
            app.client
                .get(app.url(&format!("/api/blobs/uploads/{upload_id}")))
                .header("Authorization", format!("Bearer {outsider_token}"))
                .send()
                .await
                .expect("request failed"),
        ),
        (
            "finalize",
            finalize_upload(&app, &outsider_token, upload_id, 0, 0, None).await,
        ),
        (
            "owner probing an unknown id",
            app.client
                .get(app.url(&format!("/api/blobs/uploads/{}", Uuid::now_v7())))
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
                .expect("request failed"),
        ),
    ] {
        assert_eq!(
            resp.status().as_u16(),
            404,
            "{label} must render as plain absence"
        );
    }

    let resp = app
        .client
        .get(app.url(&format!("/api/blobs/uploads/{upload_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "the owner still sees their session"
    );
}

/// The home gate at begin: an unreadable home is 404 (absent), a readable-but-not-writable
/// home is 403, the owner is 200 — the F-2 two-step, at the segmented door too. (The
/// finalize re-run of the same gate is the same function at the same layer; the
/// standing-revoked-mid-upload arm has no surface to revoke with and stays declared.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn begin_refuses_under_scoped_homes(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_owner_profile, ctx, owner_token) = owner(&app.pool).await;

    let out_email = format!("staging-gate-out-{}@example.com", Uuid::new_v4());
    let (outsider, _) = fixtures::create_test_profile_with_context(&app.pool, &out_email).await;
    let outsider_token = generate_test_jwt(&format!("test|{outsider}"), &out_email);
    let resp = begin_upload(&app, &outsider_token, "kb_contexts", ctx).await;
    assert_eq!(resp.status().as_u16(), 404, "an unreadable home is absent");

    let reader_email = format!("staging-gate-ro-{}@example.com", Uuid::new_v4());
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
    let resp = begin_upload(&app, &reader_token, "kb_contexts", ctx).await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "read is strictly broader than write: a reader cannot begin staging into the home"
    );

    let resp = begin_upload(&app, &owner_token, "kb_contexts", ctx).await;
    assert_eq!(resp.status().as_u16(), 200, "the owner begins");
}

/// An unknown home TABLE is refused in the wrapper's terms at begin — the same one
/// vocabulary the single-request path speaks.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn begin_refuses_an_unknown_home_table(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, _ctx, token) = owner(&app.pool).await;
    let resp = begin_upload(&app, &token, "kb_teams", Uuid::now_v7()).await;
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("kb_contexts") && text.contains("kb_cogmaps"),
        "refusal must name the home vocabulary: {text}"
    );
}

// ─── S4 witnesses: the list + relations surfaces (the relate face of D3) ─────────────────────

/// The relate request body, in the wire's own vocabulary.
fn relate_body(
    direction: &str,
    peer_table: &str,
    peer_id: Uuid,
    label: &str,
    weight: f64,
) -> String {
    format!(
        r#"{{"direction":"{direction}","peer_table":"{peer_table}","peer_id":"{peer_id}","edge_kind":"express","polarity":"forward","label":"{label}","weight":{weight}}}"#
    )
}

fn relate(app: &TestApp, token: &str, blob: Uuid, body: String) -> reqwest::RequestBuilder {
    app.client
        .post(app.url(&format!("/api/blobs/{blob}/relations")))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
}

async fn relations_of(app: &TestApp, token: &str, blob: Uuid) -> (u16, serde_json::Value) {
    let resp = app
        .client
        .get(app.url(&format!("/api/blobs/{blob}/relations")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json body");
    (status, body)
}

/// The resource-side listing read (`GET /api/resources/{id}/edges`), the surface the
/// blob-peer widening touches.
async fn edges_of(app: &TestApp, token: &str, resource: Uuid) -> (u16, serde_json::Value) {
    let resp = app
        .client
        .get(app.url(&format!("/api/resources/{resource}/edges")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json body");
    (status, body)
}

async fn seed_witness_resource(app: &TestApp, ctx: Uuid, profile: Uuid, title: &str) -> Uuid {
    let event = common::seed_genesis_event(&app.pool, profile, ctx).await;
    common::seed_resource(&app.pool, ctx, profile, event, title, "research").await
}

/// one-blob-many-relations, at the surface the goal's actors actually drive: relating a
/// blob to a second resource neither created nor removed the first; folding one leaves the
/// other; and a re-assert upserts (same handle, new weight) rather than duplicating.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn one_blob_many_relations_come_and_go_individually(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (profile, ctx, token) = owner(&app.pool).await;

    let resp = commit_multipart(
        &app,
        &token,
        b"figure-bytes".to_vec(),
        "image/png",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    let blob: serde_json::Value = resp.json().await.expect("json body");
    let blob_id: Uuid = blob["blob_id"]
        .as_str()
        .expect("blob_id")
        .parse()
        .expect("uuid");

    let r1 = seed_witness_resource(&app, ctx, profile, "First figure target").await;
    let r2 = seed_witness_resource(&app, ctx, profile, "Second figure target").await;

    let resp = relate(
        &app,
        &token,
        blob_id,
        relate_body("blob_as_source", "kb_resources", r1, "figure_of", 1.0),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "relate #1; body: {}",
        resp.text().await.unwrap_or_default()
    );
    let handle1: Uuid = resp.json::<serde_json::Value>().await.unwrap()["edge_handle"]
        .as_str()
        .expect("edge_handle")
        .parse()
        .expect("uuid");
    let resp = relate(
        &app,
        &token,
        blob_id,
        relate_body("blob_as_source", "kb_resources", r2, "figure_of", 1.0),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let handle2: Uuid = resp.json::<serde_json::Value>().await.unwrap()["edge_handle"]
        .as_str()
        .expect("edge_handle")
        .parse()
        .expect("uuid");
    assert_ne!(handle1, handle2, "two relations, two handles");

    let (status, body) = relations_of(&app, &token, blob_id).await;
    assert_eq!(status, 200);
    let rows = body.as_array().expect("array");
    assert_eq!(rows.len(), 2, "both relations list");
    assert!(rows.iter().all(|r| r["direction"] == "outgoing"));
    assert!(rows.iter().all(|r| r["peer_table"] == "kb_resources"));
    assert!(
        rows.iter()
            .any(|r| r["peer_title"] == "First figure target"),
        "resource peers carry their title"
    );

    // Fold relation #1 through the INCUMBENT fold endpoint — retraction rides the
    // relationship machinery every edge already answers to, blob endpoints included.
    let resp = app
        .client
        .post(app.url(&format!("/api/relationships/{handle1}/fold")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({"reason": "witness: fold one of two"}))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "fold must succeed");

    let (status, body) = relations_of(&app, &token, blob_id).await;
    assert_eq!(status, 200);
    let rows = body.as_array().expect("array");
    assert_eq!(rows.len(), 1, "folding one leaves the other");
    assert_eq!(
        rows[0]["edge_id"],
        serde_json::json!(handle2),
        "the SURVIVOR is relation #2"
    );

    // Re-assert #2 with a new weight: the same handle, the weight updated — never a
    // duplicate active edge.
    let resp = relate(
        &app,
        &token,
        blob_id,
        relate_body("blob_as_source", "kb_resources", r2, "figure_of", 2.0),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let reasserted: Uuid = resp.json::<serde_json::Value>().await.unwrap()["edge_handle"]
        .as_str()
        .expect("edge_handle")
        .parse()
        .expect("uuid");
    assert_eq!(reasserted, handle2, "re-assert upserts the active edge");
    let (_status, body) = relations_of(&app, &token, blob_id).await;
    assert_eq!(
        body.as_array().expect("array").len(),
        1,
        "still exactly one live relation"
    );
}

/// The list and relations surfaces HONOR blob-visibility-self-contained — they gate on the
/// same predicate the read-through uses and never widen it: an outsider's list excludes the
/// blob, the outsider's relations read is the SAME 404 an unknown id gets, and a
/// readable-but-not-authorable reader cannot relate (the edge homes in the blob's home).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_and_relations_surfaces_honor_blob_visibility(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let resp = commit_multipart(
        &app,
        &token,
        b"visible-only-at-home".to_vec(),
        "image/png",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    let blob: serde_json::Value = resp.json().await.expect("json body");
    let blob_id: Uuid = blob["blob_id"]
        .as_str()
        .expect("blob_id")
        .parse()
        .expect("uuid");

    // The owner's list contains it; an outsider's list does not say it exists.
    let resp = app
        .client
        .get(app.url("/api/blobs"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let rows: Vec<serde_json::Value> = resp.json().await.expect("array");
    assert!(rows
        .iter()
        .any(|r| r["blob_id"] == serde_json::json!(blob_id)));

    let out_email = format!("blob-list-out-{}@example.com", Uuid::new_v4());
    let (outsider, _) = fixtures::create_test_profile_with_context(&app.pool, &out_email).await;
    let outsider_token = generate_test_jwt(&format!("test|{outsider}"), &out_email);
    let resp = app
        .client
        .get(app.url("/api/blobs"))
        .header("Authorization", format!("Bearer {outsider_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let rows: Vec<serde_json::Value> = resp.json().await.expect("array");
    assert!(
        rows.iter()
            .all(|r| r["blob_id"] != serde_json::json!(blob_id)),
        "an outsider's list never names the blob"
    );

    // Relations: the SAME 404 for an invisible blob and an unknown one (probe learns nothing).
    let (out_status, _) = relations_of(&app, &outsider_token, blob_id).await;
    assert_eq!(out_status, 404);
    let (unknown_status, _) = relations_of(&app, &outsider_token, Uuid::now_v7()).await;
    assert_eq!(
        unknown_status, 404,
        "invisible and absent are indistinguishable"
    );

    // The home-authorable gate: an explicit context READ grant can list and read relations
    // (visibility), but relating is authoring into the home → 403.
    let reader_email = format!("blob-relate-ro-{}@example.com", Uuid::new_v4());
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
    let resp = app
        .client
        .get(app.url("/api/blobs"))
        .header("Authorization", format!("Bearer {reader_token}"))
        .send()
        .await
        .expect("request failed");
    let rows: Vec<serde_json::Value> = resp.json().await.expect("array");
    assert!(
        rows.iter()
            .any(|r| r["blob_id"] == serde_json::json!(blob_id)),
        "a reader can list it"
    );
    let resp = relate(
        &app,
        &reader_token,
        blob_id,
        relate_body(
            "blob_as_source",
            "kb_resources",
            Uuid::now_v7(),
            "figure_of",
            1.0,
        ),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "read is not author: relating is authoring into the home"
    );
}

/// The relate face's refusals name their vocabulary: a malformed peer table speaks
/// `blob_relate:`; an unreadable peer is 404 without confirming it exists; an invisible
/// blob is the ordinary not-found.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn relate_refusals_name_their_vocabulary(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (profile, ctx, token) = owner(&app.pool).await;

    let resp = commit_multipart(
        &app,
        &token,
        b"vocab".to_vec(),
        "image/png",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    let blob: serde_json::Value = resp.json().await.expect("json body");
    let blob_id: Uuid = blob["blob_id"]
        .as_str()
        .expect("blob_id")
        .parse()
        .expect("uuid");

    // Malformed peer table → 400 naming the three admissible anchors.
    let resp = relate(
        &app,
        &token,
        blob_id,
        relate_body("blob_as_source", "kb_files", Uuid::now_v7(), "x", 1.0),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 400);
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("blob_relate:"),
        "the refusal speaks its vocabulary: {text}"
    );
    assert!(
        text.contains("kb_resources"),
        "and names the admissible set: {text}"
    );

    // A peer the caller cannot read → 404 that does not confirm existence: seed a resource
    // in ANOTHER profile's context and point at it.
    let other_email = format!("blob-peer-{}@example.com", Uuid::new_v4());
    let (other, other_ctx) =
        fixtures::create_test_profile_with_context(&app.pool, &other_email).await;
    let hidden = seed_witness_resource(&app, other_ctx, other, "Hidden peer").await;
    let resp = relate(
        &app,
        &token,
        blob_id,
        relate_body("blob_as_source", "kb_resources", hidden, "figure_of", 1.0),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        404,
        "an unreadable peer is absent, not forbidden"
    );
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("not found or not readable"),
        "the refusal says the same thing an unknown id would: {text}"
    );

    // An invisible BLOB is the ordinary not-found, and the body cannot even be reached
    // through a peer the caller DOES own: the blob gate runs first.
    let own = seed_witness_resource(&app, ctx, profile, "Own peer").await;
    let out_email = format!("blob-rel-out-{}@example.com", Uuid::new_v4());
    let (outsider, out_ctx) =
        fixtures::create_test_profile_with_context(&app.pool, &out_email).await;
    let outsider_token = generate_test_jwt(&format!("test|{outsider}"), &out_email);
    let outsiders_own = seed_witness_resource(&app, out_ctx, outsider, "Outsider peer").await;
    let resp = relate(
        &app,
        &outsider_token,
        blob_id,
        relate_body(
            "blob_as_source",
            "kb_resources",
            outsiders_own,
            "figure_of",
            1.0,
        ),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        404,
        "the blob gate answers before the peer is even read"
    );
    let _ = own;
}

/// The derivation-source act at the surface: resource → blob (`blob_as_target`), the exact
/// two calls the CLI's `--preserve-source` hook composes — commit the file, then relate it
/// as the resource's derivation source. The relations read is the graph read D3 promises.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_derivation_source_edge_names_the_file_a_resource_came_from(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["image/png", "application/pdf"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (profile, ctx, token) = owner(&app.pool).await;

    // "The file": committed as a blob (the CLI hook's first call).
    let resp = commit_multipart(
        &app,
        &token,
        b"%PDF-1.4 original".to_vec(),
        "application/pdf",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    let blob: serde_json::Value = resp.json().await.expect("json body");
    let blob_id: Uuid = blob["blob_id"]
        .as_str()
        .expect("blob_id")
        .parse()
        .expect("uuid");

    let resource = seed_witness_resource(&app, ctx, profile, "Derived research").await;
    // The hook's second call: relate blob-as-target so the resource's derivation source is
    // the blob.
    let resp = relate(
        &app,
        &token,
        blob_id,
        relate_body(
            "blob_as_target",
            "kb_resources",
            resource,
            "derivation_source",
            1.0,
        ),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let (status, body) = relations_of(&app, &token, blob_id).await;
    assert_eq!(status, 200);
    let rows = body.as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["direction"], "incoming",
        "the edge points AT the blob"
    );
    assert_eq!(rows[0]["label"], "derivation_source");
    assert_eq!(
        rows[0]["peer_title"], "Derived research",
        "the graph read names the resource"
    );
}

// ─── The resource-side listing renders blob peers (the S6 follow-up, task 01a061d4) ────────

/// The widened resource-side listing: the `derivation_source` edge answers "what is this
/// resource derived from" from the RESOURCE side — the blob peer rides as peer_table + bare
/// id, no title — while a resource↔resource edge renders unchanged beside it.
///
/// And the negative face at the surface the widening touches. The fixture splits the homes:
/// the blob (and therefore the edge — relate homes on the BLOB's home) lives in a second
/// profile's context, the resource in the first profile's. The resource's owner — who CAN
/// see the resource — gets no trace of the derivation edge: the row is absent, not
/// redacted. That is structural today because blob-relate is the ONLY writer of blob-ended
/// edges (the generic assert path fixes its endpoint table at `kb_resources`,
/// db_backend `assert_edge_from_source_home`); if a future surface ever homes
/// resource→blob edges on the resource, this face rides `edges_visible_to`'s
/// both-endpoints arms instead — the equivalence oracle pins those independently.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_resource_side_listing_renders_blob_peers_and_hides_invisible_ones(pool: PgPool) {
    let cfg = blob_cfg(1 << 20, &["application/pdf"], 64 * 1024);
    let app = blob_app(pool, cfg).await;
    let (resource_owner, resource_ctx, resource_owner_token) = owner(&app.pool).await;
    let resource =
        seed_witness_resource(&app, resource_ctx, resource_owner, "Derived research").await;

    // A second profile owns the blob's home. They are granted read on the resource's
    // context — that is what lets the relate's peer gate pass — and nothing of theirs
    // (blob or edge) is reachable from the resource owner's standing.
    let blob_owner_email = format!("blob-edge-owner-{}@example.com", Uuid::new_v4());
    let (blob_owner, blob_ctx) =
        fixtures::create_test_profile_with_context(&app.pool, &blob_owner_email).await;
    let blob_owner_token = generate_test_jwt(&format!("test|{blob_owner}"), &blob_owner_email);
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, false, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(resource_ctx)
    .bind(blob_owner)
    .execute(&app.pool)
    .await
    .expect("grant read on the resource's context");

    // The blob is committed into the BLOB OWNER's context; the derivation_source edge is
    // homed there too (relate homes on the blob's home).
    let resp = commit_multipart(
        &app,
        &blob_owner_token,
        b"%PDF-1.4 original".to_vec(),
        "application/pdf",
        "kb_contexts",
        blob_ctx,
    )
    .send()
    .await
    .expect("request failed");
    let blob: serde_json::Value = resp.json().await.expect("json body");
    let blob_id: Uuid = blob["blob_id"]
        .as_str()
        .expect("blob_id")
        .parse()
        .expect("uuid");
    let resp = relate(
        &app,
        &blob_owner_token,
        blob_id,
        relate_body(
            "blob_as_target",
            "kb_resources",
            resource,
            "derivation_source",
            1.0,
        ),
    )
    .send()
    .await
    .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "relate must land");

    // A plain resource↔resource edge beside it — the incumbent face must be unchanged.
    let other = seed_witness_resource(&app, resource_ctx, resource_owner, "Unrelated peer").await;
    let resp = app
        .client
        .post(app.url("/api/relationships"))
        .header("Authorization", format!("Bearer {resource_owner_token}"))
        .json(&serde_json::json!({
            "source": resource.to_string(),
            "target": other.to_string(),
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": "depends_on",
            "weight": 1.0
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "resource↔resource assert must land"
    );

    // The blob-home's reader sees BOTH rows: the blob peer by bare id alone, the resource
    // peer with title and slug — polymorphism in one listing.
    let (status, body) = edges_of(&app, &blob_owner_token, resource).await;
    assert_eq!(status, 200);
    let rows = body.as_array().expect("array");
    assert_eq!(
        rows.len(),
        2,
        "blob-home reader sees the derivation edge AND the resource edge: {body}"
    );
    let derivation = rows
        .iter()
        .find(|r| r["label"] == "derivation_source")
        .expect("derivation_source row");
    assert_eq!(derivation["peer_table"], "kb_blobs");
    assert_eq!(derivation["peer_id"], serde_json::json!(blob_id));
    assert!(
        derivation["peer_title"].is_null(),
        "a blob peer has no title: {derivation}"
    );
    assert!(
        derivation["peer_slug"].is_null(),
        "a blob peer has no slug: {derivation}"
    );
    assert_eq!(
        derivation["direction"], "outgoing",
        "the queried resource is the edge's source"
    );
    let res_edge = rows
        .iter()
        .find(|r| r["label"] == "depends_on")
        .expect("resource edge row");
    assert_eq!(res_edge["peer_table"], "kb_resources");
    assert_eq!(res_edge["peer_title"], "Unrelated peer");
    assert_eq!(res_edge["peer_slug"], "unrelated-peer");

    // The resource's owner — who can see the resource, but has no standing in the blob's
    // home — sees only the resource edge. The fact of the derivation relation renders
    // nowhere: not as a row, not as a redaction, not as an error.
    let (status, body) = edges_of(&app, &resource_owner_token, resource).await;
    assert_eq!(status, 200);
    let rows = body.as_array().expect("array");
    assert_eq!(
        rows.len(),
        1,
        "a reader without the blob's home sees only the resource edge: {body}"
    );
    assert_eq!(rows[0]["label"], "depends_on");
    assert!(
        !body.to_string().contains(&blob_id.to_string()),
        "an edge to an unreadable blob never names the blob: {body}"
    );
}

// ─── F8: the commit door's transport bound is the config's threshold, not axum's 2 MB ──

/// A multipart commit between axum's inherited 2 MB default and the D7 threshold must
/// SUCCEED: mounted plain, the door inherited the app-wide 2 MB transport default and a
/// legal 2–4 MB upload died as a misleading `malformed multipart body` instead of the
/// threshold vocabulary. The limit is derived from the config, so a legal body reaches
/// the handler and an over-threshold body hears the threshold refusal, never the
/// transport's.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_commit_between_the_old_transport_default_and_the_threshold_succeeds(pool: PgPool) {
    let threshold = 4 * 1024 * 1024;
    let cfg = blob_cfg(8 * 1024 * 1024, &["application/octet-stream"], threshold);
    let app = blob_app(pool, cfg).await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    // Between 2 MB (the old inherited default) and the 4 MB threshold.
    let bytes: Vec<u8> = (0..2_500_000u32).map(|i| (i % 251) as u8).collect();
    let resp = commit_multipart(
        &app,
        &token,
        bytes.clone(),
        "application/octet-stream",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    assert_eq!(
        status, 200,
        "a legal over-2MB commit must not die at the transport; body: {body}"
    );
}

// ─── F9: a 5xx from a blob door names the door, never the provider ──────────────

/// A provider whose `put` bails with provider-shaped text (status + response body, the
/// `VercelBlobStore` failure shape). The commit must render a 500 whose body carries NO
/// byte of the provider's response — the crate's own `From<sqlx::Error>` scrub, applied
/// to the blob doors: the full error goes to the log, the wire names the door.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_provider_bail_is_scrubbed_from_the_wire(pool: PgPool) {
    struct FailingStore;

    impl std::fmt::Debug for FailingStore {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FailingStore")
        }
    }

    #[async_trait::async_trait]
    impl temper_substrate::blob_store::BlobStore for FailingStore {
        async fn exists(&self, _pathname: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn put(
            &self,
            _pathname: &str,
            _content_type: &str,
            _body: bytes::Bytes,
            _cache_control_max_age: u32,
        ) -> anyhow::Result<temper_substrate::blob_store::PutReceipt> {
            anyhow::bail!(
                "blob provider put: 503 Service Unavailable: {{\"error\":{{\"code\":\"store_maintenance\",\"message\":\"SECRET-PROVIDER-TEXT the bucket is draining\"}}}}"
            );
        }
        async fn get(
            &self,
            _pathname: &str,
            _consistent: bool,
        ) -> anyhow::Result<temper_substrate::blob_store::ByteStream> {
            unreachable!("the commit path never reads")
        }
        async fn head(
            &self,
            _pathname: &str,
        ) -> anyhow::Result<Option<temper_substrate::blob_store::BlobHead>> {
            unreachable!("the commit path never heads")
        }
    }

    let cfg = blob_cfg(1 << 20, &["image/png"], 64 * 1024);
    let app = setup_test_app_with_state(pool, move |state| {
        state.blob_store = Some(Arc::new(FailingStore));
        let mut config = (*state.config).clone();
        config.blob = Some(cfg);
        state.config = Arc::new(config);
    })
    .await;
    let (_profile, ctx, token) = owner(&app.pool).await;

    let resp = commit_multipart(
        &app,
        &token,
        b"bytes".to_vec(),
        "image/png",
        "kb_contexts",
        ctx,
    )
    .send()
    .await
    .expect("request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(status, 500, "a provider bail is a 500; body: {body}");
    let wire = serde_json::to_string(&body).expect("body string");
    assert!(
        !wire.contains("SECRET-PROVIDER-TEXT") && !wire.contains("store_maintenance"),
        "the provider's own response text must never reach the wire; body was {wire}"
    );
    // [widened on main — f3f5a80f] a 5xx body now carries the GENERIC internal message; even
    // the door context stays server-side (tracing carries it, the wire carries nothing). The
    // scrub property this witness guards is stronger, not weaker: the provider's text AND the
    // door context are both absent, and the constant message is what an operator's log search
    // correlates against.
    assert_eq!(
        body["error"]["message"], "An internal error occurred",
        "a 5xx renders the generic internal message; body was {wire}"
    );
}
