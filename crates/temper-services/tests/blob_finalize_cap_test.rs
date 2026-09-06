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
//!
//! The allowlist carries the identical witness set (task 01a0723c): it is the same shape
//! of rule with the same consequence, and before the pre-check landed, a commit refused
//! on its media type left the object at its content-addressed pathname with no `kb_blobs`
//! row — invisible to every surface, unreachable by any ledger-driven enumeration. The
//! witnesses here pin: a refused commit leaves the provider holding nothing (single-request),
//! a finalize refused on a mid-upload-tightened allowlist keeps both the store empty and
//! the staging resumable (the cap witness's own scenario), and begin refuses a media type
//! that could never commit before any byte is staged.
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
    blob_cfg_with_allowlist(max_bytes, &["image/png"])
}

fn blob_cfg_with_allowlist(max_bytes: i64, allowlist: &[&str]) -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes,
        allowlist: allowlist.iter().map(|s| s.to_string()).collect(),
        single_request_max_bytes: 1 << 20,
    }
}

/// The minimum the standing two-step + resolve_emitter require (the emitter-test
/// fixture's shape, inlined so this target keeps no cross-target harness dep).
async fn seed_commit_fixture(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let profile_id = Uuid::now_v7();
    let handle = format!("cap-order-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind("cap-order@example.com")
        .bind("cap-order@example.com")
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
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
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// FAILS IF: finalize puts the assembled whole to the provider before any cap decision —
/// the wrapper's refusal then leaves orphan bytes at the content-addressed pathname that
/// erasure can never reach through the ledger.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn finalize_refuses_an_over_cap_whole_before_the_provider_put(pool: sqlx::PgPool) {
    let (profile_id, context_id) = seed_commit_fixture(&pool).await;
    let caller = ProfileId::from(profile_id);

    // Stage exactly the BEGIN-time cap (1024): two 512-byte segments, both legitimately
    // landed. Then the operator's number DROPS to 512 — the staged whole is now over-cap.
    let store = InMemoryBlobStore::default();
    let upload_id = blob_service::begin_upload(
        &pool,
        &blob_cfg(1024),
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

/// FAILS IF: the single-request path puts bytes to the provider before the allowlist is
/// consulted. This was the state of the world before 01a0723c: a commit refused on its
/// media type still left the object at its content-addressed pathname — no `kb_blobs` row
/// will ever point at it, no ledger-driven enumeration can reach it, and the ordinary
/// `.docx` upload hit it as readily as the adversarial one. The refusal's vocabulary is
/// the wrapper's, byte for byte.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_refused_commit_leaves_the_provider_holding_nothing(pool: sqlx::PgPool) {
    let (profile_id, context_id) = seed_commit_fixture(&pool).await;
    let caller = ProfileId::from(profile_id);
    let store = InMemoryBlobStore::default();

    let bytes = b"plain text bytes".to_vec();
    let err = match blob_service::commit_blob(
        &pool,
        &store,
        &blob_cfg(1 << 20),
        blob_service::BlobCommitCommand {
            caller,
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(context_id.to_string()),
            content_type: "text/plain".to_string(),
            bytes: bytes.clone().into(),
            surface: Surface::ApiHttp,
        },
    )
    .await
    {
        Err(e) => e,
        Ok(_) => panic!("text/plain is outside the allowlist and must be refused"),
    };

    match err {
        ApiError::BadRequest(msg) => {
            // The wrapper's RAISE, restated byte-for-byte: the type, the rule, and the
            // allowlist in force — the operator's own values.
            assert_eq!(
                msg,
                "blob_commit: content_type text/plain is not admitted — the allowlist in \
                 force is image/png",
                "the refusal speaks the wrapper's exact vocabulary"
            );
        }
        other => panic!("the refusal is the allowlist's BadRequest, got: {other:?}"),
    }

    // The leak this task exists to close, witnessed absent: nothing at the pathname the
    // put WOULD have taken, and no ledger row either.
    let content_hash = temper_core::hash::sha256_hex(&bytes);
    assert!(
        !store.contains(&blob_pathname(&content_hash)),
        "a refused commit must leave the provider holding nothing"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_blobs")
        .fetch_one(&pool)
        .await
        .expect("count kb_blobs");
    assert_eq!(rows, 0, "a refused commit commits nothing");
}

/// FAILS IF: finalize puts the assembled whole to the provider before the allowlist is
/// consulted. The scenario is the cap witness's own: the operator TIGHTENS the allowlist
/// mid-upload, so a session begun and legitimately filled under one configuration meets
/// the wrapper's refusal at finalize — and that refusal must not cost a provider object.
/// The staging survives (resumable — the keep-and-declare posture every other finalize
/// refusal answers to).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_finalize_refused_on_a_tightened_allowlist_keeps_the_store_empty(pool: sqlx::PgPool) {
    let (profile_id, context_id) = seed_commit_fixture(&pool).await;
    let caller = ProfileId::from(profile_id);
    let store = InMemoryBlobStore::default();

    let upload_id = blob_service::begin_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        temper_substrate::payloads::AnchorRef::context(temper_core::types::ids::ContextId::from(
            context_id,
        )),
        "image/png".to_string(),
    )
    .await
    .expect("begin upload");
    let segment = vec![7u8; 512];
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        upload_id,
        0,
        segment.clone().into(),
    )
    .await
    .expect("first segment lands");
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        upload_id,
        1,
        segment.clone().into(),
    )
    .await
    .expect("second segment lands");

    // The operator tightens the allowlist mid-upload: image/png is no longer admitted.
    let err = match blob_service::finalize_upload(
        &pool,
        &store,
        &blob_cfg_with_allowlist(1024, &["image/webp"]),
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
        Ok(_) => panic!("a type the tightened allowlist refuses must not commit"),
    };

    match err {
        ApiError::BadRequest(msg) => {
            assert!(
                msg.contains("image/png is not admitted") && msg.contains("image/webp"),
                "the refusal names the type and the allowlist now in force: {msg}"
            );
        }
        other => panic!("the refusal is the allowlist's BadRequest, got: {other:?}"),
    }

    let mut whole = segment.clone();
    whole.extend_from_slice(&segment);
    let content_hash = temper_core::hash::sha256_hex(&whole);
    assert!(
        !store.contains(&blob_pathname(&content_hash)),
        "a finalize refused on the allowlist never reaches the provider"
    );
    let landed = temper_substrate::uploads::landed_segments(&pool, caller, upload_id)
        .await
        .expect("read the staging")
        .expect("the staging is kept");
    assert_eq!(landed.len(), 2, "the refusal kept the staging (resumable)");
}

/// FAILS IF: begin accepts a media type the allowlist can never admit. Before the
/// begin-time courtesy, such a session could be filled to the staging ceiling across many
/// requests before finalize refused it — wasted staging, no recourse but the reaper. The
/// refusal speaks the wrapper's vocabulary, and no session row is minted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn begin_refuses_a_media_type_that_could_never_commit(pool: sqlx::PgPool) {
    let (profile_id, context_id) = seed_commit_fixture(&pool).await;
    let caller = ProfileId::from(profile_id);

    let err = match blob_service::begin_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        temper_substrate::payloads::AnchorRef::context(temper_core::types::ids::ContextId::from(
            context_id,
        )),
        "application/zip".to_string(),
    )
    .await
    {
        Err(e) => e,
        Ok(_) => panic!("a type the allowlist refuses must not open a staging session"),
    };

    match err {
        ApiError::BadRequest(msg) => {
            assert_eq!(
                msg,
                "blob_commit: content_type application/zip is not admitted — the allowlist \
                 in force is image/png",
                "the begin-time refusal speaks the wrapper's exact vocabulary"
            );
        }
        other => panic!("the refusal is the allowlist's BadRequest, got: {other:?}"),
    }

    let sessions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_blob_uploads WHERE owner_profile_id = $1")
            .bind(profile_id)
            .fetch_one(&pool)
            .await
            .expect("count sessions");
    assert_eq!(sessions, 0, "no session row is minted for a refused begin");
}
