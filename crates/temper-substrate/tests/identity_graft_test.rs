#![cfg(feature = "artifact-tests")]
//! WS6 collapse: the grafted identity/infra layer resolves against the substrate.
//!
//! Task A folds the operational identity/auth/infra layer into the artifact — `kb_profiles` gains
//! `email`/`preferences`, the 3 infra enums (`join_request_status`/`invitation_status`/
//! `transfer_status`) and the 7 infra tables land in `01_schema.sql`, and `has_system_access` /
//! `is_system_admin` land in `02_functions.sql` (the legacy `kb_teams.is_active` predicate dropped —
//! that column does not exist in the substrate). These are additive: nothing references them until the
//! surface ports land (Tasks B–E); the legacy `public` copies are untouched.
//!
//! Each test runs on an ephemeral `public`-schema database via `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`.

mod common;

/// Seed a clean artifact (01+02), the singleton `kb_system_settings (access_mode='open')`, and one
/// profile; assert the two grafted system-access functions evaluate (open mode grants any profile),
/// `kb_profiles` carries `email`/`preferences`, and each of the 7 grafted infra tables is queryable.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn identity_graft_resolves(pool: sqlx::PgPool) {
    // Reset to clean 01+02 baseline — L0 kernel migration seeds kb_system_settings(id=1).
    common::reset_schema(&pool).await;
    // The instance-access singleton in 'open' mode, plus one profile to gate.
    sqlx::query("INSERT INTO kb_system_settings (id, access_mode) VALUES (1, 'open')")
        .execute(&pool)
        .await
        .expect("seed kb_system_settings");
    let profile = common::insert_profile(&pool, "ada").await;

    // The two grafted system-access functions evaluate; open mode grants any profile.
    let has: bool = sqlx::query_scalar("SELECT has_system_access($1)")
        .bind(profile)
        .fetch_one(&pool)
        .await
        .expect("has_system_access runs");
    assert!(has, "open access_mode grants access to any profile");

    // is_system_admin resolves (false here — no gating team / owner membership seeded).
    let _is_admin: bool = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(profile)
        .fetch_one(&pool)
        .await
        .expect("is_system_admin runs");

    // kb_profiles gained the re-added email/preferences columns.
    sqlx::query("SELECT email, preferences FROM kb_profiles LIMIT 1")
        .execute(&pool)
        .await
        .expect("kb_profiles.email/preferences resolve");

    // Each of the 7 grafted infra tables is queryable.
    for table in [
        "kb_profile_auth_links",
        "kb_system_settings",
        "kb_join_requests",
        "kb_team_invitations",
        "kb_transfers",
        "kb_blob_files",
        "kb_ingestion_records",
    ] {
        sqlx::query(&format!("SELECT 1 FROM {table} LIMIT 1"))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("grafted table {table} is queryable: {e}"));
    }
}
