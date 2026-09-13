//! Integration test — the commit-time scope-of-engagement disclosure (ruled 2026-09-12
//! with Pete; decision `01a097ff-0aa1-7183-bd57-7044286123e8`, task
//! `01a09628-9e9f-75b1-86be-a7677bc7183b`): when a profile commits bytes into a context
//! governed by ANOTHER profile, the commit door tells them, at the moment of writing,
//! what that means — bytes committed into another's context live and die with that
//! estate, and an erasure of its owner strikes them. The mitigation sits at the commit,
//! deliberately not in the erasure act.
//!
//! One witness, three arms: a guest's commit into someone else's governed context
//! carries the line; the owner's own commit carries none (the owner owes themselves no
//! disclosure); a commit into a team-owned home carries none (the team line is the
//! terms' other half, not this disclosure). Bites if the helper is never called, is
//! inverted, or leaks onto the caller's own home.
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

/// A substrate profile, its emitter entities (the write path resolves them), and a
/// profile-owned governed context (the `blob_home_exclusion_test` shape).
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

/// The owner's write grant to the guest on the owner's governed context — the real
/// authorship the commit door's standing gate resolves (`profile_explicit_grant`).
async fn grant_guest_write(pool: &PgPool, owner: Uuid, guest: Uuid, context: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, \
                principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, $3)",
    )
    .bind(context)
    .bind(guest)
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed guest write grant");
}

/// A context owned by the profile's personal team (the trigger created the team at profile
/// insert, with the profile as its `owner` member — the authorship arm the standing gate
/// resolves). Team governance, not a governed home.
async fn seed_personal_team_context(pool: &PgPool, profile: Uuid) -> Uuid {
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         SELECT $1, 'kb_teams', t.id, 'shared', 'Shared' \
           FROM kb_teams t \
          WHERE t.slug = 'personal-' || (SELECT handle FROM kb_profiles WHERE id = $2)",
    )
    .bind(context_id)
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed team context");
    context_id
}

async fn commit_bytes(
    pool: &PgPool,
    store: &InMemoryBlobStore,
    home_table: &str,
    home: Uuid,
    caller: Uuid,
    bytes: &'static [u8],
) -> temper_services::services::blob_service::BlobCommitOutcome {
    blob_service::commit_blob(
        pool,
        store,
        &blob_cfg(),
        blob_service::BlobCommitCommand {
            caller: ProfileId::from(caller),
            home_table: Some(home_table.to_string()),
            home_id: Some(home.to_string()),
            content_type: "image/png".to_string(),
            bytes: Bytes::from_static(bytes),
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("the commit lands")
}

/// ── WITNESS: the disclosure rides the commit outcome exactly when the home is another's
/// governed context ──
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_guest_commit_into_a_governed_context_discloses_the_estate_line(pool: PgPool) {
    let store = InMemoryBlobStore::default();
    let (owner, home) = seed_profile_with_context(&pool, "estate-owner@example.test").await;
    let (guest, guests_own_home) =
        seed_profile_with_context(&pool, "estate-guest@example.test").await;
    let (team_member, _) = seed_profile_with_context(&pool, "estate-teammate@example.test").await;
    let team_home = seed_personal_team_context(&pool, team_member).await;
    grant_guest_write(&pool, owner, guest, home).await;

    // THE ARM: a guest commits into the owner's governed context — the line rides the
    // outcome, at the moment of writing.
    let committed = commit_bytes(
        &pool,
        &store,
        "kb_contexts",
        home,
        guest,
        b"estate-disclosure guest bytes",
    )
    .await;
    let line = committed
        .estate_scope_disclosure
        .expect("the guest's commit discloses the scope of engagement");
    assert!(
        line.contains("live and die with that context") && line.contains("erasure of its owner"),
        "the disclosure states the estate line, got: {line}"
    );

    // The owner's own commit into the same context: no disclosure.
    let own = commit_bytes(
        &pool,
        &store,
        "kb_contexts",
        home,
        owner,
        b"estate-disclosure owner bytes",
    )
    .await;
    assert_eq!(
        own.estate_scope_disclosure, None,
        "the owner owes themselves no disclosure"
    );

    // The guest's commit into their OWN context: no disclosure.
    let guests_own = commit_bytes(
        &pool,
        &store,
        "kb_contexts",
        guests_own_home,
        guest,
        b"estate-disclosure guest-own bytes",
    )
    .await;
    assert_eq!(
        guests_own.estate_scope_disclosure, None,
        "committing into your own context is not the estate line"
    );

    // A commit into a TEAM-owned home: no disclosure — the team line is the terms' other
    // half and is not this commit's note. The committer is the team's owner-member, the
    // authorship arm the standing gate resolves.
    let team_commit = commit_bytes(
        &pool,
        &store,
        "kb_contexts",
        team_home,
        team_member,
        b"estate-disclosure team bytes",
    )
    .await;
    assert_eq!(
        team_commit.estate_scope_disclosure, None,
        "the team-home commit is the team line, not the estate disclosure"
    );

    // The SEGMENTED door speaks the same line: the guest's staged upload into the owner's
    // governed context carries the disclosure at finalize, where the blob is born.
    let upload_id = blob_service::begin_upload(
        &pool,
        &blob_cfg(),
        ProfileId::from(guest),
        temper_substrate::payloads::AnchorRef::context(temper_core::types::ids::ContextId::from(
            home,
        )),
        "image/png".to_string(),
    )
    .await
    .expect("the guest begins a staged upload");
    blob_service::append_to_upload(
        &pool,
        &blob_cfg(),
        ProfileId::from(guest),
        upload_id,
        0,
        Bytes::from_static(b"estate-disclosure segmented bytes"),
    )
    .await
    .expect("the segment lands");
    let finalized = blob_service::finalize_upload(
        &pool,
        &store,
        &blob_cfg(),
        ProfileId::from(guest),
        upload_id,
        &temper_core::types::blob::BlobUploadFinalizeRequest {
            expected_segments: 1,
            expected_total_bytes: 33,
            expected_content_hash: None,
        },
        Surface::Mcp,
    )
    .await
    .expect("the staged upload finalizes");
    let segment_line = finalized
        .estate_scope_disclosure
        .expect("the segmented door discloses at the moment of writing");
    assert!(
        segment_line.contains("live and die with that context"),
        "the segmented door carries the same line: {segment_line}"
    );
}
