//! Persistence for `kb_machine_clients` — the machine-principal allowlist.

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use sqlx::PgPool;
    use uuid::Uuid;

    const BACKFILL: &str =
        include_str!("../../../../migrations/20260711000011_backfill_machine_clients.sql");

    /// Seed a profile plus an `auth0-m2m` auth link, as prod carries for the steward.
    async fn seed_agent_link(pool: &PgPool, client_id: &str) -> Uuid {
        let profile_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, $2, $3, NULL, '{}')",
            profile_id,
            format!("agent-{client_id}"),
            format!("agent-{client_id}"),
        )
        .execute(pool)
        .await
        .expect("seed profile");

        sqlx::query!(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at) \
             VALUES ($1, $2, 'auth0-m2m', $3, NULL, false, true, now())",
            Uuid::now_v7(),
            profile_id,
            client_id,
        )
        .execute(pool)
        .await
        .expect("seed auth link");

        profile_id
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn backfill_registers_existing_m2m_links_and_is_idempotent(pool: PgPool) {
        let profile_id = seed_agent_link(&pool, "steward-client-1").await;

        sqlx::raw_sql(BACKFILL)
            .execute(&pool)
            .await
            .expect("backfill runs");

        let row = sqlx::query!(
            "SELECT profile_id, registered_by_profile_id, label, issuer, revoked_at \
               FROM kb_machine_clients WHERE client_id = $1",
            "steward-client-1",
        )
        .fetch_one(&pool)
        .await
        .expect("backfilled row exists");

        assert_eq!(row.profile_id, profile_id);
        assert_eq!(
            row.registered_by_profile_id, profile_id,
            "backfilled rows are self-registered: no human authorized them (D13)"
        );
        assert!(row.label.starts_with("backfilled: "));
        assert_eq!(row.issuer, "auth0-m2m");
        assert!(row.revoked_at.is_none());

        // Re-running is a no-op, not a duplicate-key error.
        sqlx::raw_sql(BACKFILL)
            .execute(&pool)
            .await
            .expect("backfill is idempotent");
        let count = sqlx::query_scalar!("SELECT count(*) FROM kb_machine_clients")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, Some(1));
    }
}
