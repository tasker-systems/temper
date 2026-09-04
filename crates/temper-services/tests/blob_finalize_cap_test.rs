//! Integration test — finalize's cap ordering (review F4's leak face): an over-cap
//! assembled whole can never exist to reach the provider, so the cap check runs BEFORE
//! the provider put, never after.
//!
//! The per-append staging ceiling makes an over-cap whole unreachable through the front
//! door; the one way it can still exist is an operator LOWERING the cap mid-upload —
//! bytes already staged above the new cap, finalize then assembling and PUTTING them
//! before the SQL wrapper's commit-time cap refused. That put-before-refusal orphan is
//! exactly the leak the security review named: this witness pins the ordering — the
//! refusal fires, the provider holds nothing, and the staging survives (resumable).
#![cfg(feature = "test-db")]

use std::sync::Arc;

use uuid::Uuid;

use temper_core::types::blob::BlobUploadFinalizeRequest;
use temper_core::types::ids::ProfileId;
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::error::ApiError;
use temper_services::services::blob_service;
use temper_substrate::blob_store::{blob_pathname, InMemoryBlobStore};
use temper_workflow::operations::Surface;

fn blob_cfg(max_bytes: i64) -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 1 << 20,
    }
}

/// FAILS IF: finalize puts the assembled whole to the provider before any cap decision —
/// the wrapper's refusal then leaves orphan bytes at the content-addressed pathname that
/// erasure can never reach through the ledger.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn finalize_refuses_an_over_cap_whole_before_the_provider_put(pool: sqlx::PgPool) {
    // The minimum the standing two-step + resolve_emitter require (the emitter-test
    // fixture's shape, inlined so this target keeps no cross-target harness dep).
    let profile_id = Uuid::now_v7();
    let handle = format!("cap-order-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind("cap-order@example.com")
        .bind("cap-order@example.com")
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
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("seed context");
    let caller = ProfileId::from(profile_id);

    // Stage exactly the BEGIN-time cap (1024): two 512-byte segments, both legitimately
    // landed. Then the operator's number DROPS to 512 — the staged whole is now over-cap.
    let store = InMemoryBlobStore::default();
    let upload_id = blob_service::begin_upload(
        &pool,
        caller,
        temper_substrate::payloads::AnchorRef::context(temper_core::types::ids::ContextId::from(
            context_id,
        )),
        "image/png".to_string(),
    )
    .await
    .expect("begin upload");
    let segment = vec![9u8; 512];
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        upload_id,
        0,
        segment.clone().into(),
    )
    .await
    .expect("first segment lands under the begin-time cap");
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        upload_id,
        1,
        segment.clone().into(),
    )
    .await
    .expect("second segment lands at exactly the begin-time cap");

    let err = match blob_service::finalize_upload(
        &pool,
        &store,
        &blob_cfg(512),
        caller,
        upload_id,
        &BlobUploadFinalizeRequest {
            expected_segments: 2,
            expected_total_bytes: 1024,
            expected_content_hash: None,
        },
        Surface::Mcp,
    )
    .await
    {
        Err(e) => e,
        Ok(_) => panic!("an over-cap whole is refused before anything is uploaded"),
    };

    match err {
        ApiError::BadRequest(msg) => {
            assert!(
                msg.contains("per-blob cap"),
                "the refusal names the cap in force: {msg}"
            );
            assert!(
                msg.contains("nothing was uploaded"),
                "the refusal says what did not happen: {msg}"
            );
        }
        other => panic!("the refusal is the cap's BadRequest, got: {other:?}"),
    }

    // The leak the review named, witnessed absent: nothing at the pathname the put would
    // have taken — and the staging survives the refusal (keep-and-declare, resumable).
    let mut whole = segment.clone();
    whole.extend_from_slice(&segment);
    let content_hash = temper_core::hash::sha256_hex(&whole);
    let pathname = blob_pathname(&content_hash);
    assert!(
        !store.contains(&pathname),
        "an over-cap whole never reaches the provider"
    );
    let landed = temper_substrate::uploads::landed_segments(&pool, caller, upload_id)
        .await
        .expect("read the staging")
        .expect("the staging is kept");
    assert_eq!(landed.len(), 2, "the refusal kept the staging in place");
}
