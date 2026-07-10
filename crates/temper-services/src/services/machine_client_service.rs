//! Persistence for `kb_machine_clients` — the machine-principal allowlist.
//!
//! Read path (`lookup_by_client_id`, `touch_last_seen`) is on the authentication
//! hot path for every machine call. Write paths are admin-driven and rare.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::MachineClient;

use crate::error::{ApiError, ApiResult};

/// The authentication-path lookup. `None` ⇒ unregistered. A revoked row still
/// resolves here; the caller distinguishes (the gate needs the timestamp to
/// build a useful rejection message).
pub async fn lookup_by_client_id(
    pool: &PgPool,
    client_id: &str,
) -> ApiResult<Option<MachineClient>> {
    let row = sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients
            WHERE client_id = $1"#,
        client_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Coarse liveness touch (D9): writes only when `last_seen_at` is NULL or older
/// than five minutes, so the common authentication is a pure read. Returns
/// whether a write actually happened.
pub async fn touch_last_seen(pool: &PgPool, id: Uuid) -> ApiResult<bool> {
    let result = sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET last_seen_at = now()
            WHERE id = $1
              AND (last_seen_at IS NULL OR last_seen_at < now() - interval '5 minutes')"#,
        id,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Load one machine client by its own id.
pub async fn get(pool: &PgPool, id: Uuid) -> ApiResult<MachineClient> {
    sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Enumerate registered clients, newest first. Revoked rows are hidden unless asked for.
pub async fn list(pool: &PgPool, include_revoked: bool) -> ApiResult<Vec<MachineClient>> {
    let rows = sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients
            WHERE $1 OR revoked_at IS NULL
            ORDER BY created DESC"#,
        include_revoked,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mark a client dead. Idempotent in effect but not in record: a second revoke of an
/// already-revoked row is a no-op that returns the existing row (the first revoker and
/// first timestamp are the truth). Grants and memberships are deliberately untouched (D11).
pub async fn revoke(pool: &PgPool, id: Uuid, revoker: ProfileId) -> ApiResult<MachineClient> {
    sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *revoker,
    )
    .execute(pool)
    .await?;
    get(pool, id).await
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use sqlx::PgPool;
    use uuid::Uuid;

    use crate::services::machine_client_service as svc;
    use temper_core::types::ids::ProfileId;

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

    /// Register `client_id` against a freshly seeded agent profile.
    async fn seed_registered(pool: &PgPool, client_id: &str) -> Uuid {
        let profile_id = seed_agent_link(pool, client_id).await;
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_machine_clients (id, client_id, label, profile_id, registered_by_profile_id) \
             VALUES ($1, $2, 'test', $3, $3)",
            id,
            client_id,
            profile_id,
        )
        .execute(pool)
        .await
        .expect("seed machine client");
        id
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn lookup_finds_registered_and_misses_unregistered(pool: PgPool) {
        seed_registered(&pool, "known").await;

        let hit = svc::lookup_by_client_id(&pool, "known")
            .await
            .expect("lookup");
        assert!(hit.is_some(), "registered client resolves");
        assert_eq!(hit.expect("some").client_id, "known");

        let miss = svc::lookup_by_client_id(&pool, "never-registered")
            .await
            .expect("lookup");
        assert!(miss.is_none(), "unregistered client must not resolve");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn touch_last_seen_is_coarse(pool: PgPool) {
        let id = seed_registered(&pool, "coarse").await;

        // First touch writes (last_seen_at was NULL).
        assert!(svc::touch_last_seen(&pool, id).await.expect("touch 1"));

        // Second touch, immediately after, does NOT write: the row is inside the
        // five-minute window. This is what keeps authentication read-only (D9).
        assert!(
            !svc::touch_last_seen(&pool, id).await.expect("touch 2"),
            "two authentications inside five minutes must produce one write"
        );

        // Age the row past the window; the next touch writes again.
        sqlx::query!(
            "UPDATE kb_machine_clients SET last_seen_at = now() - interval '6 minutes' WHERE id = $1",
            id,
        )
        .execute(&pool)
        .await
        .expect("age row");
        assert!(svc::touch_last_seen(&pool, id).await.expect("touch 3"));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoke_marks_dead_and_list_hides_by_default(pool: PgPool) {
        let id = seed_registered(&pool, "doomed").await;
        let admin = seed_agent_link(&pool, "admin-actor").await;

        let revoked = svc::revoke(&pool, id, ProfileId::from(admin))
            .await
            .expect("revoke");
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoked_by_profile_id, Some(admin));

        let active = svc::list(&pool, false).await.expect("list active");
        assert!(active.iter().all(|c| c.client_id != "doomed"));

        let all = svc::list(&pool, true).await.expect("list all");
        assert!(all.iter().any(|c| c.client_id == "doomed"));
    }
}
