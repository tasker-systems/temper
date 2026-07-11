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

/// The longest a rotated-away secret may remain valid. A rotation window is meant to be brief
/// (issue new → deploy → old expires); an unbounded grace would keep a possibly-compromised old
/// secret alive far past the rotation's intent, defeating D6's "two live secrets, briefly".
const MAX_ROTATION_GRACE_SECONDS: i64 = 7 * 24 * 3_600;

/// Rotate a temper-issued secret (Phase B1, D6). Moves the current secret to `previous` with a
/// grace window, installs a fresh current, and returns the new plaintext once. Rejects a client
/// that temper did not issue (its secret lives at its IdP), one already revoked, or a grace
/// window outside `[0, MAX_ROTATION_GRACE_SECONDS]`.
pub async fn rotate_secret(
    pool: &PgPool,
    id: Uuid,
    grace_seconds: i64,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    if !(0..=MAX_ROTATION_GRACE_SECONDS).contains(&grace_seconds) {
        return Err(ApiError::BadRequest(format!(
            "grace_seconds must be between 0 and {MAX_ROTATION_GRACE_SECONDS} (7 days); got {grace_seconds}"
        )));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // FOR UPDATE locks the row so a concurrent revoke cannot land between these guards and the
    // write — the issuer/revoked checks and the rotation see one consistent, pinned row.
    let row = sqlx::query!(
        "SELECT issuer, client_id, revoked_at FROM kb_machine_clients WHERE id = $1 FOR UPDATE",
        id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.issuer != "temper" {
        return Err(ApiError::BadRequest(format!(
            "machine client '{}' was not issued by temper (issuer '{}'); its secret is managed by its IdP",
            row.client_id, row.issuer
        )));
    }
    if row.revoked_at.is_some() {
        return Err(ApiError::BadRequest(format!(
            "machine client '{}' is revoked; issue a new credential instead",
            row.client_id
        )));
    }

    let secret = crate::auth::secret::mint_secret();
    sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET secret_hash_previous       = secret_hash,
                  secret_previous_expires_at = now() + make_interval(secs => $2),
                  secret_hash                = $3,
                  secret_rotated_at          = now()
            WHERE id = $1"#,
        id,
        grace_seconds as f64,
        secret.hash,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    let client = get(pool, id).await?;
    Ok(temper_core::types::machine::IssuedMachineCredential {
        client,
        client_secret: secret.plaintext,
    })
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

    /// Seed a temper-issued client with a known secret hash. Returns the machine_client id.
    async fn seed_temper_issued(pool: &PgPool, client_id: &str, secret: &str) -> Uuid {
        let profile_id = seed_agent_link(pool, client_id).await;
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_machine_clients \
               (id, client_id, issuer, label, profile_id, registered_by_profile_id, secret_hash) \
             VALUES ($1, $2, 'temper', 'test', $3, $3, $4)",
            id,
            client_id,
            profile_id,
            crate::auth::secret::sha256_hex(secret),
        )
        .execute(pool)
        .await
        .expect("seed temper-issued");
        id
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rotate_secret_moves_current_to_previous_with_expiry(pool: PgPool) {
        let id = seed_temper_issued(&pool, "tmpr_rot", "old-secret").await;

        let cred = svc::rotate_secret(&pool, id, 3600).await.expect("rotate");

        // A fresh plaintext is returned and its hash is the new current.
        let row = sqlx::query!(
            "SELECT secret_hash, secret_hash_previous, secret_previous_expires_at, secret_rotated_at \
               FROM kb_machine_clients WHERE id = $1",
            id,
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row.secret_hash.as_deref(),
            Some(crate::auth::secret::sha256_hex(&cred.client_secret).as_str()),
            "current is the new secret"
        );
        assert_eq!(
            row.secret_hash_previous.as_deref(),
            Some(crate::auth::secret::sha256_hex("old-secret").as_str()),
            "previous is the old secret"
        );
        // The grace expiry is now()+grace, computed by make_interval — assert the math, not just
        // presence (a wrong unit or an `as f64` surprise would pass an is_some() check).
        let expiry = row
            .secret_previous_expires_at
            .expect("previous has a grace expiry");
        let delta = (expiry - chrono::Utc::now()).num_seconds();
        assert!(
            (3540..=3660).contains(&delta),
            "previous expiry is ~now()+3600s, got {delta}s"
        );
        assert!(row.secret_rotated_at.is_some(), "rotation is stamped");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rotate_secret_rejects_out_of_range_grace(pool: PgPool) {
        let id = seed_temper_issued(&pool, "tmpr_grace", "s").await;
        assert!(
            matches!(
                svc::rotate_secret(&pool, id, -1)
                    .await
                    .expect_err("negative grace"),
                crate::error::ApiError::BadRequest(_)
            ),
            "a negative grace is rejected"
        );
        assert!(
            matches!(
                svc::rotate_secret(&pool, id, 999_999_999)
                    .await
                    .expect_err("excessive grace"),
                crate::error::ApiError::BadRequest(_)
            ),
            "a grace past the 7-day cap is rejected (keeps an old secret alive too long)"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rotate_secret_rejects_a_non_temper_issued_client(pool: PgPool) {
        // A plain auth0-m2m registration (issuer default), no secret.
        let id = seed_registered(&pool, "auth0-client").await;

        let err = svc::rotate_secret(&pool, id, 3600)
            .await
            .expect_err("must reject");
        assert!(
            matches!(err, crate::error::ApiError::BadRequest(_)),
            "auth0-m2m secrets are managed by the IdP, not temper; got {err:?}"
        );
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
