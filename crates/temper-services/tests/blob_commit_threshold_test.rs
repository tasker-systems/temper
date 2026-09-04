//! Integration test — the D7 single-request threshold is enforced on the SHARED commit
//! seam (review F5 of the adversarial security record, `01a063f9`).
//!
//! The MCP commit tool reaches `blob_service::commit_blob` with no threshold gate of its
//! own — three places PROMISE the refusal (the tool module doc, the tool description, the
//! service's own module doc) while the only enforcement lived in the HTTP multipart
//! handler, so an MCP client could single-shot-commit ~3.5× the threshold every other
//! committing surface honors, with full decode and provider put before any size decision.
//! The gate now lives in the seam every committing surface shares, FIRST — before the home
//! parse, before standing, before any byte reaches the provider.
#![cfg(feature = "test-db")]

use std::sync::Arc;

use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::error::ApiError;
use temper_services::services::blob_service;
use temper_substrate::blob_store::InMemoryBlobStore;
use temper_workflow::operations::Surface;

/// A test-sized `BlobConfig`: static-token posture (no env races), a tiny threshold so the
/// over-threshold arm is a handful of bytes.
fn blob_cfg() -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes: 1 << 20,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 16,
    }
}

/// FAILS IF: any committing surface — the MCP tool included, which rides this seam with no
/// gate of its own — can single-shot-commit bytes past the single-request threshold. The
/// `home_table` is deliberately INVALID (kb_resources is not a blob home): the refusal that
/// comes back must be the THRESHOLD's, proving the gate outranks the home parse — and the
/// empty provider proves no byte was put while any of it was being decided.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_commit_seam_refuses_over_threshold_bytes_before_anything_else(pool: sqlx::PgPool) {
    let cfg = blob_cfg();
    let store = InMemoryBlobStore::default();
    let bytes = vec![7u8; 32]; // twice the threshold

    let err = match blob_service::commit_blob(
        &pool,
        &store,
        &cfg,
        blob_service::BlobCommitCommand {
            caller: temper_core::types::ids::ProfileId::from(uuid::Uuid::now_v7()),
            home_table: Some("kb_resources".to_string()),
            home_id: Some(uuid::Uuid::now_v7().to_string()),
            content_type: "image/png".to_string(),
            bytes: bytes.clone().into(),
            surface: Surface::Mcp,
        },
    )
    .await
    {
        Err(e) => e,
        Ok(_) => panic!("over-threshold bytes are refused at the seam"),
    };

    match err {
        ApiError::BadRequest(msg) => {
            assert!(
                msg.contains("single-request threshold"),
                "the refusal names the threshold in force: {msg}"
            );
            assert!(
                msg.contains("segmented upload path"),
                "the refusal names the path beyond the threshold: {msg}"
            );
        }
        other => panic!("the refusal is the threshold's BadRequest, got: {other:?}"),
    }

    // No byte reached the provider while any of it was being decided.
    let content_hash = temper_core::hash::sha256_hex(&bytes);
    let pathname = temper_substrate::blob_store::blob_pathname(&content_hash);
    assert!(
        !store.contains(&pathname),
        "an over-threshold commit must never reach the provider"
    );
}

/// The threshold is a CEILING, not a floor: at exactly the threshold the commit proceeds —
/// the same number the MCP read ceiling is keyed to, so the two cannot silently drift.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn bytes_at_exactly_the_threshold_still_commit(pool: sqlx::PgPool) {
    let cfg = blob_cfg();

    // Seed the minimum the standing two-step + resolve_emitter require (the emitter-test
    // fixture's shape, inlined here so this target keeps no cross-target harness dep).
    let profile_id = uuid::Uuid::now_v7();
    let handle = format!("threshold-ok-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind("threshold-ok@example.com")
        .bind("threshold-ok@example.com")
        .execute(&pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(&pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("seed context");

    let outcome = blob_service::commit_blob(
        &pool,
        &InMemoryBlobStore::default(),
        &cfg,
        blob_service::BlobCommitCommand {
            caller: temper_core::types::ids::ProfileId::from(profile_id),
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(context_id.to_string()),
            content_type: "image/png".to_string(),
            bytes: vec![7u8; 16].into(), // exactly the threshold
            surface: Surface::Mcp,
        },
    )
    .await
    .expect("bytes at exactly the threshold commit");

    assert!(!outcome.deduped, "fresh bytes are not a dedup hit");
}
