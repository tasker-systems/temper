//! Integration test — the per-owner staging bounds (ruled 2026-09-24 on task
//! `01a0723e-cfe5-7080-9e0e-9b3323c25080`): the per-session ceiling bounds one session,
//! and nothing bounded how many sessions a principal could hold open or how many bytes
//! they could stage across them — resident in the operational Postgres every other
//! surface shares.
//!
//! The ruled bounds fire at `begin_upload` and derive from the ONE operator knob: a
//! principal holds at most 8 open sessions, and stages at most 5 × `BLOB_MAX_BYTES`
//! across them, so raising the cap is one deliberate operator act and the byte budget
//! scales with it. The refusals speak the incumbent `blob_upload:` vocabulary, naming
//! the bound in force and the remedy.
//!
//! The witnesses here pin:
//! - a principal at the session limit is refused a further begin (and the refusal mints
//!   no row), while an append to an already-open session inside the bound is unaffected;
//! - the budget refuses only when open staged bytes EXCEED 5 × the cap — a principal
//!   sitting exactly at the budget may still begin one more (empty) session;
//! - a mid-upload RAISE of the cap lets an open session continue at its next append —
//!   the staging ceiling is per-append against the config in force, never begin-time.

#![cfg(feature = "test-db")]

use std::sync::Arc;

use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::error::ApiError;
use temper_services::services::blob_service;

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

/// The minimum the standing two-step requires (the sibling test target's fixture shape,
/// inlined so this target keeps no cross-target harness dep).
async fn seed_begin_fixture(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let profile_id = Uuid::now_v7();
    let handle = format!("owner-bounds-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind("owner-bounds@example.com")
        .bind("owner-bounds@example.com")
        .execute(pool)
        .await
        .expect("seed profile");
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

async fn begin(pool: &sqlx::PgPool, caller: ProfileId, context_id: Uuid) -> Result<Uuid, ApiError> {
    blob_service::begin_upload(
        pool,
        &blob_cfg(1024),
        caller,
        temper_substrate::payloads::AnchorRef::context(temper_core::types::ids::ContextId::from(
            context_id,
        )),
        "image/png".to_string(),
    )
    .await
}

/// FAILS IF: a principal can open staging sessions without bound. At the ruled limit of
/// 8 open sessions the ninth begin refuses in the `blob_upload:` vocabulary, naming the
/// bound in force — and mints no row. An append to an already-open session inside the
/// bound is UNAFFECTED: the bound gates beginning, never an upload in flight.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn begin_refuses_when_the_principal_holds_too_many_open_sessions(pool: sqlx::PgPool) {
    let (profile_id, context_id) = seed_begin_fixture(&pool).await;
    let caller = ProfileId::from(profile_id);

    let mut first_id = None;
    for _ in 0..8 {
        let id = begin(&pool, caller, context_id)
            .await
            .expect("sessions under the limit begin");
        first_id.get_or_insert(id);
    }

    let err = match begin(&pool, caller, context_id).await {
        Err(e) => e,
        Ok(_) => panic!("a ninth open session is refused"),
    };
    match err {
        ApiError::BadRequest(msg) => {
            assert!(
                msg.starts_with("blob_upload:"),
                "the refusal speaks the staging vocabulary: {msg}"
            );
            assert!(
                msg.contains("8 open upload sessions"),
                "the refusal names the bound in force: {msg}"
            );
            assert!(
                msg.contains("finalize") || msg.contains("abandon"),
                "the refusal names the remedy: {msg}"
            );
        }
        other => panic!("the refusal is the sessions bound's BadRequest, got: {other:?}"),
    }

    let sessions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_blob_uploads WHERE owner_profile_id = $1")
            .bind(profile_id)
            .fetch_one(&pool)
            .await
            .expect("count sessions");
    assert_eq!(sessions, 8, "a refused begin mints no row");

    // Criterion: an append to an already-open session inside the bound is unaffected.
    let first_id = first_id.expect("at least one open session");
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        first_id,
        0,
        vec![1u8; 64].into(),
    )
    .await
    .expect("an append inside the bound lands");
}

/// FAILS IF: begin ignores the bytes already staged across a principal's open sessions.
/// The budget is 5 × the cap (1024 → 5120 here). Five sessions staged to exactly the cap
/// put the principal exactly AT the budget — which does not exceed it, so one more (empty)
/// begin is allowed — and staging that sixth session full crosses the budget, so the
/// seventh begin refuses, naming the budget, the cap it derives from, and the knob.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn begin_refuses_when_open_staging_exceeds_the_budget(pool: sqlx::PgPool) {
    let (profile_id, context_id) = seed_begin_fixture(&pool).await;
    let caller = ProfileId::from(profile_id);

    // Five sessions, each staged to exactly the per-session cap: 5 × 1024 == budget.
    for i in 0..5 {
        let id = begin(&pool, caller, context_id)
            .await
            .expect("sessions under the limit begin");
        blob_service::append_to_upload(
            &pool,
            &blob_cfg(1024),
            caller,
            id,
            0,
            vec![i as u8; 1024].into(),
        )
        .await
        .expect("each session stages exactly its cap");
    }

    // At exactly the budget, one more begin is allowed — the bound refuses EXCEEDING.
    let sixth = begin(&pool, caller, context_id)
        .await
        .expect("a begin at exactly the budget is not a violation");
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        sixth,
        0,
        vec![9u8; 1024].into(),
    )
    .await
    .expect("the sixth session stages its cap — 6144 bytes, over the 5120 budget");

    // Six sessions held is still under the session limit, so the budget is the bound
    // that fires here.
    let err = match begin(&pool, caller, context_id).await {
        Err(e) => e,
        Ok(_) => panic!("a begin past the staging budget is refused"),
    };
    match err {
        ApiError::BadRequest(msg) => {
            assert!(
                msg.starts_with("blob_upload:"),
                "the refusal speaks the staging vocabulary: {msg}"
            );
            assert!(
                msg.contains("6144") && msg.contains("5120"),
                "the refusal names the staged total and the budget in force: {msg}"
            );
            assert!(
                msg.contains("BLOB_MAX_BYTES"),
                "the refusal names the knob the budget derives from: {msg}"
            );
        }
        other => panic!("the refusal is the budget's BadRequest, got: {other:?}"),
    }
}

/// The staging ceiling is enforced per-append, from the config in force at THAT append —
/// never frozen at begin. A session filled to its begin-time cap continues when the
/// operator RAISES the cap mid-upload (the mirror of the finalize witness's lowered-cap
/// scenario): the over-ceiling append refuses under the old config, and the same seq
/// lands under the raised one.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_mid_upload_raise_of_the_cap_lets_an_open_session_continue(pool: sqlx::PgPool) {
    let (profile_id, context_id) = seed_begin_fixture(&pool).await;
    let caller = ProfileId::from(profile_id);
    let upload_id = begin(&pool, caller, context_id)
        .await
        .expect("begin upload");

    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        upload_id,
        0,
        vec![1u8; 512].into(),
    )
    .await
    .expect("first segment lands");
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        upload_id,
        1,
        vec![2u8; 512].into(),
    )
    .await
    .expect("second segment lands at exactly the ceiling");

    let err = match blob_service::append_to_upload(
        &pool,
        &blob_cfg(1024),
        caller,
        upload_id,
        2,
        vec![3u8; 512].into(),
    )
    .await
    {
        Err(e) => e,
        Ok(_) => panic!("an append over the begin-time ceiling refuses"),
    };
    match err {
        ApiError::BadRequest(msg) => {
            assert!(
                msg.contains("blob_upload:") && msg.contains("1024"),
                "the over-ceiling refusal names the ceiling in force: {msg}"
            );
        }
        other => panic!("the refusal is the ceiling's BadRequest, got: {other:?}"),
    }

    // The operator raises the cap mid-upload: the SAME session continues at its next
    // append — the ceiling is read from the config each append carries.
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(2048),
        caller,
        upload_id,
        2,
        vec![3u8; 512].into(),
    )
    .await
    .expect("the raised cap lets the open session continue");
    let progress = blob_service::upload_progress(&pool, caller, upload_id)
        .await
        .expect("read progress");
    assert_eq!(
        progress.total_bytes, 1536,
        "the raised-cap append landed on the open session"
    );
}
