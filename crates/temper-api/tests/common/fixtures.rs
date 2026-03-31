//! Test data setup helpers.
//!
//! `clean_and_seed` removes test-created data each run (while preserving the
//! migration-seeded System / Anonymous profiles) and inserts a seed resource
//! owned by the System profile so that visibility tests have stable data.

use sqlx::PgPool;

// Well-known UUIDs from the R2 seed migration.
pub const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
pub const TEMPER_CONTEXT_ID: &str = "00000000-0000-0000-0003-000000000001";
pub const RESEARCH_DOC_TYPE_ID: &str = "00000000-0000-0000-0001-000000000004";

/// Delete all test-generated data, then create a stable seed resource owned
/// by the System profile.
///
/// Preserves the System and Anonymous profiles inserted by migrations.
pub async fn clean_and_seed(pool: &PgPool) {
    // Delete in reverse FK order. Leave kb_doc_types, kb_contexts,
    // and the two seed profiles intact.
    sqlx::query(
        "DELETE FROM kb_events WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean kb_events");

    sqlx::query("DELETE FROM kb_device_sync_state")
        .execute(pool)
        .await
        .expect("clean kb_device_sync_state");

    sqlx::query("DELETE FROM kb_transfers")
        .execute(pool)
        .await
        .expect("clean kb_transfers");

    sqlx::query("DELETE FROM kb_team_invitations")
        .execute(pool)
        .await
        .expect("clean kb_team_invitations");

    sqlx::query("DELETE FROM kb_team_resources")
        .execute(pool)
        .await
        .expect("clean kb_team_resources");

    sqlx::query("DELETE FROM kb_team_members")
        .execute(pool)
        .await
        .expect("clean kb_team_members");

    sqlx::query("DELETE FROM kb_teams")
        .execute(pool)
        .await
        .expect("clean kb_teams");

    // Remove test resources (not the seed ones if we re-run).
    sqlx::query(
        "DELETE FROM kb_resources WHERE owner_profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test resources");

    // Remove test profiles (keep System + Anonymous).
    sqlx::query(
        "DELETE FROM kb_profile_auth_links WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test auth links");

    sqlx::query(
        "DELETE FROM kb_profiles WHERE id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test profiles");

    // Seed one stable research resource owned by System profile.
    // Use upsert to handle concurrent test setup racing on both id and origin_uri.
    sqlx::query(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES (
            '00000000-0000-0000-0099-000000000001',
            $1, $2,
            'test://seed-resource',
            'Seed Research Doc',
            'seed-research-doc',
            $3, $3,
            true, now(), now()
        )
        ON CONFLICT (id) DO UPDATE SET updated = now()
        "#,
    )
    .bind(uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap())
    .bind(uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap())
    .bind(uuid::Uuid::parse_str(SYSTEM_PROFILE_ID).unwrap())
    .execute(pool)
    .await
    .expect("seed resource");
}
