//! Integration test — the blob-home exclusion (ruled 2026-09-11; decision
//! `01a092c7-76ef-7d11-80d5-7eaf56723dfb`, task
//! `01a092c8-46d8-7193-9364-4e8435d099aa`, goal
//! `01a07684-8baf-7a72-a6aa-8549a7635f04`): a blob homes in a context, never in a map.
//!
//! Two witnesses, each at the layer it bites:
//! - the DOOR: a commit naming a cogmap as home is refused in the door's own vocabulary
//!   (bites against the service change — before it, the commit proceeded to standing and
//!   could succeed for an authorable map);
//! - the SCHEMA: the `kb_blobs_home_context_only` CHECK refuses a cogmap home even for a
//!   direct SQL writer (bites against the migration — before it, the UPDATE succeeded).
//!
//! Deliberately NOT asserted here: the read floor's kb_cogmaps arm and the erasure
//! sweep's 'independent_obligation … team or map' outcome — both stay as
//! defense-in-depth per the decision, and no fixture can reach them anymore.
#![cfg(feature = "test-db")]

use std::sync::Arc;

use bytes::Bytes;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::services::blob_service;
use temper_substrate::blob_store::InMemoryBlobStore;
use temper_workflow::operations::Surface;

// ── fixtures ────────────────────────────────────────────────────────────────────────

fn blob_cfg() -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes: 1 << 20,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 64 * 1024,
    }
}

/// Seed a substrate profile, its emitter entities (the write path resolves them), and a
/// profile-owned context (the `blob_delete_door_test` shape).
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (Uuid, Uuid) {
    let profile_id = Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
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

async fn commit_blob(
    pool: &PgPool,
    store: &InMemoryBlobStore,
    home_table: &str,
    home: Uuid,
    caller: Uuid,
) -> Result<
    temper_services::services::blob_service::BlobCommitOutcome,
    temper_services::error::ApiError,
> {
    blob_service::commit_blob(
        pool,
        store,
        &blob_cfg(),
        blob_service::BlobCommitCommand {
            caller: ProfileId::from(caller),
            home_table: Some(home_table.to_string()),
            home_id: Some(home.to_string()),
            content_type: "image/png".to_string(),
            bytes: Bytes::from_static(b"blob-home-exclusion bytes"),
            surface: Surface::ApiHttp,
        },
    )
    .await
}

// ── the witnesses ───────────────────────────────────────────────────────────────────

/// The door: a commit naming a cogmap as home is refused in the door's own vocabulary,
/// before standing is ever consulted — every committing surface flows through
/// `home_gate_tables`, so the voice is one voice.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_commit_naming_a_cogmap_home_is_refused_at_the_door(pool: PgPool) {
    let store = InMemoryBlobStore::default();
    let (caller, _context) = seed_profile_with_context(&pool, "exclusion-door@example.test").await;

    let err = match commit_blob(&pool, &store, "kb_cogmaps", Uuid::now_v7(), caller).await {
        Err(err) => err,
        Ok(_) => panic!("the cogmap-home commit is refused"),
    };

    match &err {
        temper_services::error::ApiError::BadRequest(msg) => {
            assert!(
                msg.contains("a cogmap is not a blob home"),
                "the refusal speaks the door's own vocabulary, got: {msg}"
            );
        }
        other => panic!("expected a BadRequest refusal, got: {other:?}"),
    }
}

/// The schema: `kb_blobs_home_context_only` refuses a cogmap home even for a direct SQL
/// writer — the door's backstop. Exercised on a LIVE row (the committed fixture's own),
/// which is also the strike's shape: home columns survive the D5.2 emptying, so the
/// constraint covers struck rows too.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_schema_refuses_a_cogmap_homed_blob_row(pool: PgPool) {
    let store = InMemoryBlobStore::default();
    let (caller, context) = seed_profile_with_context(&pool, "exclusion-sql@example.test").await;

    let committed = commit_blob(&pool, &store, "kb_contexts", context, caller)
        .await
        .expect("the context-home commit lands");

    let update_err = sqlx::query("UPDATE kb_blobs SET home_table = 'kb_cogmaps' WHERE id = $1")
        .bind(committed.blob_id.uuid())
        .execute(&pool)
        .await
        .expect_err("the CHECK refuses the cogmap home");
    let db = update_err
        .as_database_error()
        .expect("a CHECK violation is a database error");
    assert_eq!(
        db.constraint(),
        Some("kb_blobs_home_context_only"),
        "the refusing constraint is the exclusion's own, got: {db}"
    );
}
