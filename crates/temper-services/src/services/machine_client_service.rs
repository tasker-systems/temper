//! Persistence for `kb_machine_clients` — the machine-principal allowlist.
//!
//! Read path (`lookup_by_client_id`, `touch_last_seen`) is on the authentication
//! hot path for every machine call. Write paths are operator-driven and rare, and they
//! authorize through `authz::MachineClientControlAuthority` against the *existing row's* owning
//! team (B2 D5): a system admin, or the owner of the team that owns the machine. A caller outside
//! that authority is refused exactly as a missing id is (`MACHINE_CLIENT_REFUSAL`).

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::MachineClient;

use temper_principal::{Act, ActorAuthority, Standing};

use crate::auth::AuthenticatedProfile;
use crate::authz::{MachineClientControlAuthority, Principal};
use crate::error::{ApiError, ApiResult};
use crate::services::standing_service::{self, ApplyStandingParams};

/// The refusal for a machine client the caller cannot see — absent, or present but outside the
/// caller's authority. One sentence for both, on purpose: `get` renders it for a missing row and
/// `authz::MachineClientControlAuthority` renders it for a denied one, so a caller probing ids
/// cannot tell the two apart. A second literal would reopen that oracle the first time either one
/// is "improved".
pub(crate) const MACHINE_CLIENT_REFUSAL: &str = "machine client not found or not readable";

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

/// Is this **profile** a registered, unrevoked machine principal?
///
/// The authorization-time twin of [`lookup_by_client_id`], keyed on the profile rather than the
/// `client_id`: by the time a gate runs, the `client_id` is gone — `resolve_machine_from_claims`
/// exchanged it for `client.profile_id` and every downstream layer carries only that
/// (`profile_service.rs:230-262`). Set 5's audit gate needs the same fact at a *scoped* decision, so
/// it asks here rather than re-deriving it from claims it does not have.
///
/// `revoked_at IS NULL` is not optional: a revoked row still resolves in
/// [`lookup_by_client_id`] (the authentication gate wants the timestamp for its message), so a
/// lookup that forgot the predicate would readmit a revoked machine at every site but the door.
///
/// **Not** an authentication check and not a substitute for one — a profile that never
/// authenticates cannot reach any caller of this. It answers only "is this principal one of the
/// registered agents?", which is the conjunct spec §7 names alongside readability.
pub async fn is_registered_principal(pool: &PgPool, profile: ProfileId) -> ApiResult<bool> {
    let registered: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM kb_machine_clients
                WHERE profile_id = $1
                  AND revoked_at IS NULL
           ) AS "registered!: bool""#,
        *profile,
    )
    .fetch_one(pool)
    .await?;
    Ok(registered)
}

/// Is this profile a machine principal at all — any `kb_machine_clients` row, **revoked or not**?
///
/// The question a ceiling asks, as distinct from [`is_registered_principal`]'s "may it act now?":
/// revoking a credential ends the login, not what the profile is, so a revoked machine still never
/// holds a governing role. Reads the SQL `is_machine_profile` so the services and the triggers
/// that back them (20261017100000_machines_never_govern.sql) share one definition.
pub async fn is_machine_profile(pool: &PgPool, profile: ProfileId) -> ApiResult<bool> {
    let machine: bool = sqlx::query_scalar!(
        r#"SELECT is_machine_profile($1) AS "machine!: bool""#,
        *profile,
    )
    .fetch_one(pool)
    .await?;
    Ok(machine)
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
    .ok_or_else(|| ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string()))
}

/// [`get`], authorized for a surface caller (B2 D5). [`get`] itself stays unauthorized: it is the
/// internal primitive the authentication path and the post-insert readbacks use, and gating it
/// would break them.
///
/// A caller outside the machine's authority is refused exactly as a missing id is
/// (`MACHINE_CLIENT_REFUSAL`) — see `authz::MachineClientControlAuthority`.
pub async fn get_for_caller(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
    id: Uuid,
) -> ApiResult<MachineClient> {
    authorize_control(pool, authed, id).await?;
    get(pool, id).await
}

/// The per-row gate every act on an existing machine client passes first: a system admin, or the
/// owner of the machine's owning team, keyed on the row (B2 D5). A refusal is indistinguishable
/// from a missing id. Ahead of any write, so a rejected act never touches the row.
async fn authorize_control(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
    id: Uuid,
) -> ApiResult<()> {
    crate::authz::authorize::<MachineClientControlAuthority>(pool, Principal::Proof(authed), id)
        .await?;
    Ok(())
}

/// List machine clients visible to `caller` (B2 D5), newest first. Revoked rows are hidden
/// unless asked for. A system admin sees every row, including teamless ones; a team owner sees
/// only machines owned by a team they own.
///
/// `EXISTS`, not `array_agg` — an empty scope must DENY, and an aggregate over an empty scope
/// yields NULL, which falls open.
pub async fn list(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
    include_revoked: bool,
) -> ApiResult<Vec<MachineClient>> {
    let caller = ProfileId::from(authed.profile().id);
    let is_admin = crate::services::access_service::is_system_admin(pool, caller).await?;

    let rows = sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients mc
            WHERE ($1 OR mc.revoked_at IS NULL)
              AND ( $2
                    OR ( mc.team_id IS NOT NULL
                         AND EXISTS (
                             SELECT 1
                               FROM kb_team_members tm
                              WHERE tm.team_id = mc.team_id
                                AND tm.profile_id = $3
                                AND tm.role = 'owner'
                         ) ) )
            ORDER BY created DESC"#,
        include_revoked,
        is_admin,
        *caller,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mark a client dead. Idempotent in effect but not in record: a second revoke of an
/// already-revoked row is a no-op that returns the existing row (the first revoker and
/// first timestamp are the truth). Grants and memberships are deliberately untouched (D11).
pub async fn revoke(
    pool: &PgPool,
    id: Uuid,
    authed: &AuthenticatedProfile,
) -> ApiResult<MachineClient> {
    let revoker = ProfileId::from(authed.profile().id);
    // Auth before writes, keyed on the existing row's owning team (B2 D5).
    authorize_control(pool, authed, id).await?;
    let existing = get(pool, id).await?;

    sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *revoker,
    )
    .execute(pool)
    .await?;

    // D17 — one revocation fact, not two that can drift. `revoked_at` is an AUTHENTICATION detail (a
    // revoked credential is rejected at profile resolution and never reaches admission); standing is
    // the ADMISSION fact. If they disagree an audit reads badly — "credential dead" on one axis,
    // "admitted" on the other — so a credential revocation also revokes standing.
    //
    // Load-first, not fire-and-swallow: Revoke is legal ONLY from `Approved` (transition.rs), and a
    // machine is born `Denied` (D11) and may never have been approved. Firing unconditionally would
    // fail the credential revocation on that illegal cell — but the credential is what the operator
    // asked to kill, and it must succeed regardless. Routing through `standing_service::apply` (rather
    // than an inlined committer) also means this inherits Task 15's demotion hook and Task 17's typed
    // refusal once they land. Grants and memberships are deliberately left intact (D11): Revoke denies
    // ADMISSION, which sits above them, so a later rebind still cannot silently resurrect them.
    let subject = ProfileId::from(existing.profile_id);
    if standing_service::load(pool, subject).await? == Some(Standing::Approved) {
        standing_service::apply(
            pool,
            ApplyStandingParams {
                subject,
                act: Act::Revoke {
                    reason: format!("machine client {} revoked", existing.client_id),
                },
                actor: Some(revoker),
                authority: ActorAuthority::Admin,
            },
        )
        .await?;
    }

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
    authed: &AuthenticatedProfile,
    id: Uuid,
    grace_seconds: i64,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    // Auth before writes, keyed on the existing row's owning team (B2 D5). Ahead of the
    // transaction, so a rejected rotation never touches the row.
    authorize_control(pool, authed, id).await?;

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
    .ok_or_else(|| ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string()))?;

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

    /// Seed a caller who is genuinely a system admin.
    ///
    /// B2 D5 authorizes the lifecycle inside the service against the row's owning team, and the
    /// rows these tests seed are teamless — which is admin-only (D2). So the caller can no longer
    /// be a bare profile. Under D11 admin-ness is a `kb_principal_governance` grant plus an
    /// `approved` `kb_principal_standing`, not gating-team ownership; the gating-team upsert below
    /// is retained only because `temper-system` already exists in a migrated database.
    async fn seed_admin(pool: &PgPool, handle: &str) -> ProfileId {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, $2, $2, NULL, '{}')",
            id,
            handle,
        )
        .execute(pool)
        .await
        .expect("seed admin profile");

        let team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System') \
             ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name \
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("gating team");

        sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
            .execute(pool)
            .await
            .expect("configure gating team");

        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'owner'::team_role) \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
            team,
            id,
        )
        .execute(pool)
        .await
        .expect("join gating team as owner");

        // What confers admin-ness now: approved standing (front door) + a governance grant.
        crate::test_support::approved_admin(pool, id).await;

        ProfileId::from(id)
    }

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
        let admin = seed_admin(&pool, "rot-admin").await;
        let id = seed_temper_issued(&pool, "tmpr_rot", "old-secret").await;

        let cred = svc::rotate_secret(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            id,
            3600,
        )
        .await
        .expect("rotate");

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
        let admin = seed_admin(&pool, "grace-admin").await;
        let id = seed_temper_issued(&pool, "tmpr_grace", "s").await;
        assert!(
            matches!(
                svc::rotate_secret(
                    &pool,
                    &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
                    id,
                    -1
                )
                .await
                .expect_err("negative grace"),
                crate::error::ApiError::BadRequest(_)
            ),
            "a negative grace is rejected"
        );
        assert!(
            matches!(
                svc::rotate_secret(
                    &pool,
                    &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
                    id,
                    999_999_999
                )
                .await
                .expect_err("excessive grace"),
                crate::error::ApiError::BadRequest(_)
            ),
            "a grace past the 7-day cap is rejected (keeps an old secret alive too long)"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rotate_secret_rejects_a_non_temper_issued_client(pool: PgPool) {
        let admin = seed_admin(&pool, "issuer-admin").await;
        // A plain auth0-m2m registration (issuer default), no secret.
        let id = seed_registered(&pool, "auth0-client").await;

        let err = svc::rotate_secret(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            id,
            3600,
        )
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
        let admin = seed_admin(&pool, "admin-actor").await;

        let revoked = svc::revoke(
            &pool,
            id,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
        )
        .await
        .expect("revoke");
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoked_by_profile_id, Some(*admin));

        let active = svc::list(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            false,
        )
        .await
        .expect("list active");
        assert!(active.iter().all(|c| c.client_id != "doomed"));

        let all = svc::list(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            true,
        )
        .await
        .expect("list all");
        assert!(all.iter().any(|c| c.client_id == "doomed"));
    }

    /// A machine client with genuine reach — a read grant and a team membership, the two things D11
    /// says a revoke must NOT touch. Returns the machine-client row id (what `revoke` takes), the
    /// machine's profile id, and the acting admin.
    struct MachineReach {
        machine_id: Uuid,
        machine_profile: Uuid,
        admin: ProfileId,
    }

    async fn seed_machine_with_reach(pool: &PgPool) -> MachineReach {
        let admin = seed_admin(pool, "revoke-standing-admin").await;
        let team: Uuid =
            sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
                .fetch_one(pool)
                .await
                .expect("gating team seeded by seed_admin");
        let machine_id = seed_registered(pool, "reachful-machine").await;
        let machine_profile: Uuid = sqlx::query_scalar!(
            "SELECT profile_id FROM kb_machine_clients WHERE id = $1",
            machine_id
        )
        .fetch_one(pool)
        .await
        .expect("machine profile");

        // subject_table is CHECK-constrained to the grantable object kinds; subject_id has no FK, so
        // a synthetic context id stands in for "some object this machine can read".
        sqlx::query!(
            "INSERT INTO kb_access_grants \
               (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
             VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
            Uuid::now_v7(),
            machine_profile,
            *admin,
        )
        .execute(pool)
        .await
        .expect("grant reach");

        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'watcher'::team_role) \
             ON CONFLICT (team_id, profile_id) DO NOTHING",
            team,
            machine_profile,
        )
        .execute(pool)
        .await
        .expect("member reach");

        MachineReach {
            machine_id,
            machine_profile,
            admin,
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoking_a_credential_also_revokes_standing_but_leaves_grants(pool: PgPool) {
        // D17 — one revocation fact, one place. `revoked_at` becomes purely an authentication
        // detail; standing tells the admission story; the two cannot drift because a credential
        // revocation drives both.
        let f = seed_machine_with_reach(&pool).await;
        crate::test_support::approve(&pool, f.machine_profile).await;

        let grants_before: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_access_grants WHERE principal_id = $1",
            f.machine_profile
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        let members_before: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE profile_id = $1",
            f.machine_profile
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert!(
            grants_before > 0 && members_before > 0,
            "fixture must have reach to preserve"
        );

        svc::revoke(
            &pool,
            f.machine_id,
            &crate::test_support::authenticated_profile_for(&pool, f.admin.uuid()).await,
        )
        .await
        .unwrap();

        let state: String = sqlx::query_scalar!(
            "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
            f.machine_profile
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            state, "revoked",
            "one revocation fact, not two that can drift"
        );

        // THE PRIOR INTENT, PROVEN NOT ASSUMED. Revocation deliberately leaves grants and
        // memberships so a rebind cannot silently resurrect them (D11). Revoke on standing denies
        // admission, which sits ABOVE grants and does not touch them.
        let grants_after: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_access_grants WHERE principal_id = $1",
            f.machine_profile
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        let members_after: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE profile_id = $1",
            f.machine_profile
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(grants_after, grants_before, "D11's intent survives D17");
        assert_eq!(members_after, members_before, "D11's intent survives D17");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoking_an_unapproved_machine_is_not_an_error(pool: PgPool) {
        // Revoke is illegal from `Denied` (§6 — you cannot revoke what was never granted), but a
        // credential revocation must still succeed. The standing fire is load-guarded to `Approved`
        // in exactly this cell, so the credential revocation the operator asked for never fails.
        let f = seed_machine_with_reach(&pool).await; // born Denied under D11, never approved

        svc::revoke(
            &pool,
            f.machine_id,
            &crate::test_support::authenticated_profile_for(&pool, f.admin.uuid()).await,
        )
        .await
        .expect("credential revocation must succeed even with nothing to revoke on standing");

        // The load-first guard fired nothing (this machine was never `Approved`), so no `revoked`
        // row was written — the credential revocation the operator asked for still succeeded.
        let state: Option<String> = sqlx::query_scalar!(
            "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
            f.machine_profile
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_ne!(
            state.as_deref(),
            Some("revoked"),
            "an unapproved machine has nothing to revoke on the admission axis"
        );
    }

    /// The existence oracle, closed: a caller outside a machine's authority gets the **same** error
    /// for an existing machine client as for a missing id — variant and message — on every per-row
    /// act. A `Forbidden` on any of these would mean the row exists.
    ///
    /// Four targets, because the row's state is what a mis-ordered check would leak:
    /// - teamless (admin-only, spec D2);
    /// - owned by a team the prober does not own, while the prober owns a team of their own;
    /// - revoked;
    /// - IdP-issued.
    ///
    /// The last two are what pin `rotate_secret`'s order. Move its revoked or issuer `400` above
    /// the gate and those rows answer differently from a missing id, which this test then catches.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_denied_machine_client_is_indistinguishable_from_a_missing_one(pool: PgPool) {
        async fn team_owned_by(pool: &PgPool, slug: &str, owner: Uuid) -> Uuid {
            let team: Uuid = sqlx::query_scalar(
                "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id",
            )
            .bind(slug)
            .fetch_one(pool)
            .await
            .expect("seed team");
            sqlx::query(
                "INSERT INTO kb_team_members (team_id, profile_id, role) \
                 VALUES ($1, $2, 'owner'::team_role)",
            )
            .bind(team)
            .bind(owner)
            .execute(pool)
            .await
            .expect("seed owner");
            team
        }

        let teamless = seed_temper_issued(&pool, "probe-teamless", "s3cret").await;

        let owned = seed_temper_issued(&pool, "probe-owned", "s3cret").await;
        let rightful_owner = seed_agent_link(&pool, "probe-rightful-owner").await;
        let owning_team = team_owned_by(&pool, "probe-owning-team", rightful_owner).await;
        sqlx::query("UPDATE kb_machine_clients SET team_id = $2 WHERE id = $1")
            .bind(owned)
            .bind(owning_team)
            .execute(&pool)
            .await
            .expect("give the machine an owning team");

        let revoked = seed_temper_issued(&pool, "probe-revoked", "s3cret").await;
        sqlx::query("UPDATE kb_machine_clients SET revoked_at = now() WHERE id = $1")
            .bind(revoked)
            .execute(&pool)
            .await
            .expect("revoke");

        let idp_issued = seed_registered(&pool, "probe-idp-issued").await;

        // The prober is a team owner too — just not of any team that owns a target.
        let prober_profile = seed_agent_link(&pool, "probe-agent").await;
        team_owned_by(&pool, "probe-own-team", prober_profile).await;
        let prober = crate::test_support::authenticated_profile_for(&pool, prober_profile).await;
        let missing = Uuid::now_v7();

        let targets = [
            ("teamless", teamless),
            ("owned by another team", owned),
            ("revoked", revoked),
            ("IdP-issued", idp_issued),
        ];
        for (state, target) in targets {
            for id in [target, missing] {
                let refusals = [
                    (
                        "get",
                        svc::get_for_caller(&pool, &prober, id)
                            .await
                            .expect_err("get"),
                    ),
                    (
                        "revoke",
                        svc::revoke(&pool, id, &prober).await.expect_err("revoke"),
                    ),
                    (
                        "rotate",
                        svc::rotate_secret(&pool, &prober, id, 0)
                            .await
                            .expect_err("rotate"),
                    ),
                ];
                for (act, err) in refusals {
                    assert!(
                        matches!(&err, crate::error::ApiError::NotFound(m) if m == svc::MACHINE_CLIENT_REFUSAL),
                        "{act} on a {state} target (or a missing id): must refuse as a missing \
                         machine client, got {err:?}"
                    );
                }
            }
        }

        for (state, target) in targets {
            let rotated_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
                "SELECT secret_rotated_at FROM kb_machine_clients WHERE id = $1",
            )
            .bind(target)
            .fetch_one(&pool)
            .await
            .expect("read rotation");
            assert!(
                rotated_at.is_none(),
                "{state}: the denied rotate wrote nothing"
            );
        }
        for target in [teamless, owned, idp_issued] {
            assert!(
                svc::get(&pool, target)
                    .await
                    .expect("get")
                    .revoked_at
                    .is_none(),
                "the denied revoke wrote nothing"
            );
        }
    }
}
