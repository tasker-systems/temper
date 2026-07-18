//! Transactional registration of machine principals.
//!
//! `provision` is the inversion (D3): it creates the agent profile, its auth link, its
//! emitter entities, its gating-team membership, its explicit reach, and the
//! `kb_machine_clients` row — all in ONE transaction, ahead of the machine's first call.
//!
//! Authorization happens HERE, not in the handler (B2 D3): `provision` and `issue` resolve the
//! caller's authority through `machine_authz` before opening the transaction, so a rejected
//! registration leaves the database completely unchanged. They record the authorized caller as
//! `registered_by_profile_id`. `rebind` is the exception: it is **system-admin-only** (it
//! transplants an existing profile's reach, which team ownership cannot bound — see its doc).

use sqlx::PgPool;
use temper_substrate::ids::EntityId;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{MachineClient, ProvisionMachineRequest, RebindMachineRequest};

use crate::error::{ApiError, ApiResult};
use crate::services::access_service::{self, insert_grant, InsertGrantParams};
use crate::services::machine_authz::{self, AuthorizedReach};
use crate::services::machine_client_service;
use crate::services::profile_service;

/// Enroll `profile_id` in the configured gating team as `watcher` — **but only if `caller`,
/// the minter, is a member of that gating team themselves.**
///
/// D14 is why the enrollment exists at all: `trg_sync_system_membership` auto-joins new profiles
/// ONLY while `access_mode = 'open'`, because `has_system_access` short-circuits true under that
/// mode. Under `invite_only` it enrolls nothing, and an unenrolled machine authenticates and then
/// 403s at `require_system_access`. So we enroll explicitly, exactly as
/// `access_service::review_request` does for an approved human. Never depend on the trigger: its
/// behavior is a function of a setting that is about to change.
///
/// The caller check is why it is CONTAINED. This was the one piece of a machine's reach that
/// escaped B2's containment rule — every other piece passes through `machine_authz`, which bounds
/// a non-admin minter to reach they could confer on a human. Gating-team membership is system
/// access, so an unconditional enrollment lets a minter confer access they may not hold.
///
/// Today that is unobservable and stays so: `temper-system` carries `auto_join_role = 'watcher'`
/// and prod runs `open`, so the trigger has already made EVERY profile — every minter — a
/// gating-team member, and every machine still enrolls. The check binds the day that stops
/// (auto_join_role cleared, or the instance moved off open mode): from then on a machine can only
/// hold system access if the human who minted it did. An admin is by definition an OWNER of the
/// gating team (`is_system_admin` IS that ownership), so an admin-minted machine always enrolls
/// and D14 is preserved on the path that matters most.
///
/// Note the predicate is MEMBERSHIP, not admin-ness: the ordinary minter is a plain human whom
/// auto-join made a `watcher`, and their machines must keep enrolling.
async fn enroll_in_gating_team(
    conn: &mut sqlx::PgConnection,
    caller: ProfileId,
    profile_id: Uuid,
) -> ApiResult<()> {
    let slug = sqlx::query_scalar!("SELECT gating_team_slug FROM kb_system_settings LIMIT 1")
        .fetch_optional(&mut *conn)
        .await?
        .flatten();

    let Some(slug) = slug else {
        // No gating team configured ⇒ nothing to enroll into. `update_system_settings`
        // already rejects `invite_only` with an empty slug, so this is the open-mode case.
        return Ok(());
    };

    // Auth before write. Read on the transaction's connection, so the membership we check is the
    // membership the INSERT below acts under — a concurrent removal cannot slip between them.
    let caller_is_member = sqlx::query_scalar!(
        r#"SELECT EXISTS(
             SELECT 1 FROM kb_team_members m
               JOIN kb_teams t ON t.id = m.team_id
              WHERE t.slug = $1 AND m.profile_id = $2 )"#,
        slug,
        *caller,
    )
    .fetch_one(&mut *conn)
    .await?
    .unwrap_or(false);

    if !caller_is_member {
        // Deliberate — and the only breadcrumb an operator gets. The machine registers cleanly
        // and then 403s later at `require_system_access`; say why HERE, or that 403 is a mystery.
        tracing::warn!(
            gating_team = %slug,
            caller = %*caller,
            machine_profile_id = %profile_id,
            "gating-team enrollment skipped: the minter is not a member of the gating team, so \
             the machine cannot be conferred system access its minter does not itself hold"
        );
        return Ok(());
    }

    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
           SELECT t.id, $2, 'watcher'::team_role FROM kb_teams t WHERE t.slug = $1
           ON CONFLICT (team_id, profile_id) DO NOTHING"#,
        slug,
        profile_id,
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Apply the explicit reach: team memberships and cogmap grants. Reach is plural and
/// never inferred from `owner_team_id` (D10, D6).
///
/// Takes an [`AuthorizedReach`] — which only `machine_authz` can construct — so reach can
/// never be applied without having been authorized against the caller's own authority
/// (spec D3).
///
/// The raw `insert_grant` / raw team INSERT below remain deliberately unchecked: for a system
/// admin that is Phase A's D5 bypass, and for a team owner `machine_authz::contain_reach` has
/// already proven the reach is a subset of what the caller could confer on a human. The
/// authorization is in the TYPE now, not in a comment asking you not to widen this.
async fn apply_reach(
    conn: &mut sqlx::PgConnection,
    caller: ProfileId,
    profile_id: Uuid,
    reach: AuthorizedReach<'_>,
    // `Some` iff `reach` carries at least one grant — a pure team-membership reach fires no
    // grant_created event and so needs no emitter (the caller resolves one only when there is a
    // grant to author).
    emitter: Option<EntityId>,
) -> ApiResult<()> {
    for team in reach.teams() {
        sqlx::query!(
            r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, $3::text::team_role)
               ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role"#,
            team.team_id,
            profile_id,
            team.role,
        )
        .execute(&mut *conn)
        .await?;
    }

    for grant in reach.grants() {
        insert_grant(
            &mut *conn,
            &InsertGrantParams {
                subject_table: "kb_cogmaps".to_string(),
                subject_id: grant.cogmap_id,
                principal_table: "kb_profiles".to_string(),
                principal_id: profile_id,
                // Write implies read — the DB's coherence CHECK enforces it anyway.
                can_read: true,
                can_write: grant.can_write,
                can_delete: false,
                can_grant: false,
                granted_by_profile_id: *caller,
            },
            emitter
                .expect("a grant implies a resolved emitter (Some iff reach.grants() non-empty)"),
        )
        .await?;
    }

    Ok(())
}

/// Both unique constraints a duplicate `client_id` can trip. The auth-link one fires
/// first, because `create_agent_profile_and_link` inserts before the registration row.
const DUPLICATE_CONSTRAINTS: [&str; 2] = [
    "kb_machine_clients_client_id_key",
    "kb_profile_auth_links_auth_provider_auth_provider_user_id_key",
];

/// Name the client id in a duplicate-registration conflict.
///
/// `From<sqlx::Error> for ApiError` already maps SQLSTATE 23505 to
/// `Conflict("Resource already exists")`, so this is purely about the message: an operator
/// registering a client that already exists should be told *which* one. Any other error
/// falls through to the standard mapping.
fn map_duplicate(err: sqlx::Error, client_id: &str) -> ApiError {
    if let sqlx::Error::Database(ref db) = err {
        if db
            .constraint()
            .is_some_and(|c| DUPLICATE_CONSTRAINTS.contains(&c))
        {
            return ApiError::Conflict(format!(
                "machine client '{client_id}' is already registered"
            ));
        }
    }
    ApiError::from(err)
}

/// The auth-link unique constraint fires before the registration row's; turn its Conflict
/// into a client-id-naming message.
fn map_duplicate_from_conflict(err: ApiError, client_id: &str) -> ApiError {
    match err {
        ApiError::Conflict(_) => ApiError::Conflict(format!(
            "machine client '{client_id}' is already registered"
        )),
        other => other,
    }
}

/// Register a new machine principal, creating its agent profile. One transaction.
pub async fn provision(
    pool: &PgPool,
    caller: ProfileId,
    req: &ProvisionMachineRequest,
) -> ApiResult<MachineClient> {
    // Auth before writes: a rejected registration must leave the DB completely unchanged —
    // no orphaned agent profile, no partial enrollment. Resolving before the transaction is
    // what makes that assertable.
    let reach = machine_authz::authorize_registration(
        pool,
        caller,
        req.owner_team_id,
        &req.teams,
        &req.grants,
    )
    .await?;

    // Resolve the caller's emitter BEFORE the transaction — matching the auth-before-tx pattern above,
    // and avoiding a nested pool acquire while `tx` is open. Only when there is a grant to author: a
    // pure team-membership reach fires no grant_created event, so it must not require the minter to
    // carry a `<handle>@web` entity (a mere gating-team watcher does not).
    let emitter = if reach.grants().is_empty() {
        None
    } else {
        Some(
            temper_substrate::writes::resolve_emitter(pool, caller, "web")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?,
        )
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // The friendly-conflict check. It is NOT the race guard — two concurrent provisions both
    // pass it. The unique constraints are the guard, and `map_duplicate` turns either one into
    // a 409 naming the client id.
    if machine_client_service::lookup_by_client_id(pool, &req.client_id)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(format!(
            "machine client '{}' is already registered",
            req.client_id
        )));
    }

    let (profile_id, handle) =
        profile_service::create_agent_profile_and_link(&mut tx, &req.client_id)
            .await
            .map_err(|e| map_duplicate_from_conflict(e, &req.client_id))?;

    profile_service::provision_profile_entities(&mut tx, profile_id, &handle).await?;
    enroll_in_gating_team(&mut tx, caller, profile_id).await?;
    apply_reach(&mut tx, caller, profile_id, reach, emitter).await?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_machine_clients
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id)
           VALUES ($1, 'auth0-m2m', $2, $3, $4, $5)
           RETURNING id"#,
        req.client_id,
        req.label,
        profile_id,
        req.owner_team_id,
        *caller,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &req.client_id))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    machine_client_service::get(pool, id).await
}

/// Issue a temper-minted machine credential (Phase B1). temper generates the `client_id` and
/// the secret; the SHA-256 hex of the secret is stored, the plaintext is returned once. Creates
/// the agent profile, auth link, emitters, gating-team membership, and reach — all in one
/// transaction, exactly like `provision`, but with `issuer='temper'` and a `secret_hash`.
pub async fn issue(
    pool: &PgPool,
    caller: ProfileId,
    req: &temper_core::types::machine::IssueMachineRequest,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    // Auth before writes — same reasoning as `provision`: nothing is minted, and no profile
    // is created, unless the caller may confer this reach.
    let reach = machine_authz::authorize_registration(
        pool,
        caller,
        req.owner_team_id,
        &req.teams,
        &req.grants,
    )
    .await?;

    // Resolve the caller's emitter BEFORE the transaction (see `provision`) — only when a grant is to
    // be authored, so a pure team-membership reach needs no `<handle>@web` entity on the minter.
    let emitter = if reach.grants().is_empty() {
        None
    } else {
        Some(
            temper_substrate::writes::resolve_emitter(pool, caller, "web")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?,
        )
    };

    let client_id = crate::auth::secret::mint_client_id();
    let secret = crate::auth::secret::mint_secret();

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    let (profile_id, handle) = profile_service::create_agent_profile_and_link(&mut tx, &client_id)
        .await
        .map_err(|e| map_duplicate_from_conflict(e, &client_id))?;

    profile_service::provision_profile_entities(&mut tx, profile_id, &handle).await?;
    enroll_in_gating_team(&mut tx, caller, profile_id).await?;
    apply_reach(&mut tx, caller, profile_id, reach, emitter).await?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_machine_clients
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id, secret_hash)
           VALUES ($1, 'temper', $2, $3, $4, $5, $6)
           RETURNING id"#,
        client_id,
        req.label,
        profile_id,
        req.owner_team_id,
        *caller,
        secret.hash,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &client_id))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    let client = machine_client_service::get(pool, id).await?;
    Ok(temper_core::types::machine::IssuedMachineCredential {
        client,
        client_secret: secret.plaintext,
    })
}

/// Point a fresh `client_id` at an EXISTING agent profile, revoking the old row in the
/// same transaction unless an overlap window was requested (D8).
///
/// **`rebind` is system-admin-only, unlike the rest of the machine-client lifecycle (B2).**
/// Every other endpoint merely operates on a row and can be keyed on its owning team; `rebind`
/// is the one that *transplants* an existing profile's identity — and with it, whatever reach
/// that profile already holds — onto a caller-supplied `client_id`. That reach may have been
/// conferred by an admin and can exceed a team owner's own authority, so ownership of the
/// machine's team is NOT a sufficient bar (it would let an owner inherit reach they could never
/// confer themselves, defeating B2's containment). Rebind is also the external-IdP-app-rotation
/// path (`auth0-m2m`); a team owner rotating a temper-issued credential uses `rotate_secret`.
pub async fn rebind(
    pool: &PgPool,
    caller: ProfileId,
    req: &RebindMachineRequest,
) -> ApiResult<MachineClient> {
    let old = machine_client_service::get(pool, req.from_machine_client_id).await?;

    // Auth before writes. Admin-only (see the fn doc): team ownership cannot bound the reach a
    // rebind inherits.
    if !access_service::is_system_admin(pool, caller).await? {
        return Err(ApiError::Forbidden);
    }

    // A revoked credential is dead; it must be re-created by a fresh `provision`, never
    // resurrected under a new `client_id`. Rebinding one would revive its surviving grants and
    // memberships (revoke leaves them, D11), silently undoing a deliberate revocation. Mirrors
    // `rotate_secret`'s revoked-source guard.
    if old.revoked_at.is_some() {
        return Err(ApiError::BadRequest(format!(
            "machine client '{}' is revoked; issue a new credential instead",
            old.client_id
        )));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // A second auth link for the same profile, under the new client id.
    sqlx::query!(
        r#"INSERT INTO kb_profile_auth_links
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
           VALUES ($1, $2, $3, $4, NULL, false, false, now())"#,
        Uuid::now_v7(),
        old.profile_id,
        crate::auth::MACHINE_PROVIDER_TAG,
        req.client_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &req.client_id))?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_machine_clients
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id)
           VALUES ($1, 'auth0-m2m', $2, $3, $4, $5)
           RETURNING id"#,
        req.client_id,
        req.label,
        old.profile_id,
        old.team_id,
        *caller,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &req.client_id))?;

    if !req.keep_old_active {
        sqlx::query!(
            r#"UPDATE kb_machine_clients
                  SET revoked_at = now(), revoked_by_profile_id = $2
                WHERE id = $1 AND revoked_at IS NULL"#,
            old.id,
            *caller,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    machine_client_service::get(pool, id).await
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use sqlx::PgPool;
    use uuid::Uuid;

    use temper_core::types::ids::ProfileId;
    use temper_core::types::machine::{
        GrantSpec, IssueMachineRequest, ProvisionMachineRequest, RebindMachineRequest, TeamSpec,
    };

    use crate::services::access_service;
    use crate::services::machine_registration_service as svc;

    /// Seed a caller who is genuinely a system admin.
    ///
    /// B2 D3 moved authorization out of the handler and into `provision`/`issue`, so these
    /// tests can no longer stand in a bare profile and rely on an upstream gate: the service
    /// itself now resolves the caller's authority. `is_system_admin` IS ownership of the
    /// gating team, so being an admin means being seeded as one — configure the gating team
    /// and join this profile to it as `owner`.
    ///
    /// The `temper-system` root team already exists in a migrated database (the L0 kernel
    /// migration creates it), so the team write is an upsert, not an insert.
    async fn seed_admin(pool: &PgPool) -> ProfileId {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, 'admin', 'Admin', 'admin@example.test', '{}')",
            id,
        )
        .execute(pool)
        .await
        .expect("seed admin");

        // The provisioning caller authors the grant_created events for any cogmap reach, so it must
        // carry its `<handle>@web` emitter — as a real admin does. Provision via the production path.
        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(&mut conn, id, "admin")
            .await
            .expect("provision caller emitters");
        drop(conn);

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

        ProfileId::from(id)
    }

    /// Seed a plain team owner who is NOT a system admin and holds NO gating-team membership,
    /// with the instance in the `invite_only` shape the containment guard exists for.
    ///
    /// Two trigger facts shape this fixture, and they are the reason the hole is latent rather
    /// than live. `temper-system` carries `auto_join_role = 'watcher'`, and
    /// `trg_sync_system_membership` fires on profile INSERT — so under the default `open` mode
    /// EVERY new profile is auto-joined to the gating team, minters included. That is exactly why
    /// the enrollment is harmless in today's prod, and also why this scenario is **not
    /// constructible** in open mode: the machine would be auto-joined too, and the assertion
    /// would be about the trigger rather than about our enrollment.
    ///
    /// So we flip to `invite_only` FIRST (the trigger then enrolls nothing, and the explicit
    /// enrollment is the ONLY path into the gating team — the same premise as the D14 test), and
    /// then delete any gating membership anyway, so the fixture states its precondition rather
    /// than leaning on trigger ordering.
    ///
    /// Returns the minter and the team they own — which is also the machine's owning team, the
    /// authority `machine_authz::authorize` admits them under.
    async fn seed_outsider_team_owner(
        pool: &PgPool,
        handle: &str,
        team_slug: &str,
    ) -> (ProfileId, Uuid) {
        sqlx::query!(
            "UPDATE kb_system_settings \
                SET access_mode = 'invite_only', gating_team_slug = 'temper-system'"
        )
        .execute(pool)
        .await
        .expect("invite_only with a configured gating team");

        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
            id,
            handle,
        )
        .execute(pool)
        .await
        .expect("seed minter");

        let team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id",
            team_slug,
        )
        .fetch_one(pool)
        .await
        .expect("seed team");

        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'owner'::team_role)",
            team,
            id,
        )
        .execute(pool)
        .await
        .expect("minter owns their own team");

        sqlx::query!(
            "DELETE FROM kb_team_members m USING kb_teams t \
              WHERE m.team_id = t.id AND t.slug = 'temper-system' AND m.profile_id = $1",
            id,
        )
        .execute(pool)
        .await
        .expect("the minter holds no gating-team membership");

        (ProfileId::from(id), team)
    }

    /// How many rows the profile holds in the gating team (0 or 1).
    async fn gating_memberships(pool: &PgPool, profile_id: Uuid) -> i64 {
        sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members m JOIN kb_teams t ON t.id = m.team_id \
              WHERE t.slug = 'temper-system' AND m.profile_id = $1",
            profile_id,
        )
        .fetch_one(pool)
        .await
        .expect("count gating membership")
        .unwrap_or(0)
    }

    /// B2 containment, applied to the one piece of reach that escaped it. A minter must not be
    /// able to confer system access they do not themselves hold.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_does_not_enroll_a_machine_whose_minter_is_not_in_the_gating_team(
        pool: PgPool,
    ) {
        let (alice, team) = seed_outsider_team_owner(&pool, "outsider", "acme-out").await;
        assert!(
            !access_service::is_system_admin(&pool, alice)
                .await
                .expect("is_system_admin"),
            "precondition: the minter is a plain team owner, not an admin"
        );
        assert_eq!(
            gating_memberships(&pool, *alice).await,
            0,
            "precondition: the minter holds no gating-team membership"
        );

        let mut request = req("outsider-agent");
        request.owner_team_id = Some(team);
        let client = svc::provision(&pool, alice, &request)
            .await
            .expect("a team owner may provision for their own team");

        assert_eq!(
            gating_memberships(&pool, client.profile_id).await,
            0,
            "a minter outside the gating team must not confer membership in it"
        );
        let has_access = sqlx::query_scalar!("SELECT has_system_access($1)", client.profile_id)
            .fetch_one(&pool)
            .await
            .expect("has_system_access");
        assert_eq!(
            has_access,
            Some(false),
            "the machine must not outrank its minter at the system gate"
        );
    }

    /// The same containment on `issue` — the mint path must not be a way around it.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn issue_does_not_enroll_a_machine_whose_minter_is_not_in_the_gating_team(pool: PgPool) {
        let (alice, team) = seed_outsider_team_owner(&pool, "outsider-i", "acme-out-i").await;

        let cred = svc::issue(
            &pool,
            alice,
            &IssueMachineRequest {
                label: "sidekiq".to_string(),
                owner_team_id: Some(team),
                teams: vec![],
                grants: vec![],
            },
        )
        .await
        .expect("a team owner may issue for their own team");

        assert_eq!(
            gating_memberships(&pool, cred.client.profile_id).await,
            0,
            "issue must contain gating-team reach exactly as provision does"
        );
    }

    /// The guard keys on MEMBERSHIP, not on admin-ness — which is what keeps it a no-op in
    /// today's prod. The everyday minter there is a plain human whom open-mode auto-join has made
    /// a gating-team `watcher`; their machines must still enroll, or D14 breaks for everyone but
    /// admins.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_enrolls_a_machine_whose_minter_is_a_mere_gating_team_watcher(pool: PgPool) {
        let (alice, team) = seed_outsider_team_owner(&pool, "watcher-minter", "acme-w").await;

        // Hand back exactly what open-mode auto-join hands every human today: `watcher`.
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             SELECT t.id, $1, 'watcher'::team_role FROM kb_teams t WHERE t.slug = 'temper-system'",
            *alice,
        )
        .execute(&pool)
        .await
        .expect("join the gating team as watcher");

        let mut request = req("watcher-minted-agent");
        request.owner_team_id = Some(team);
        let client = svc::provision(&pool, alice, &request)
            .await
            .expect("provision");

        assert_eq!(
            gating_memberships(&pool, client.profile_id).await,
            1,
            "a minter INSIDE the gating team still confers membership (D14)"
        );
    }

    fn req(client_id: &str) -> ProvisionMachineRequest {
        ProvisionMachineRequest {
            client_id: client_id.to_string(),
            label: "steward".to_string(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_creates_profile_link_emitters_and_registration(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let client = svc::provision(&pool, admin, &req("acme-agent"))
            .await
            .expect("provision");

        assert_eq!(client.client_id, "acme-agent");
        assert_eq!(client.issuer, "auth0-m2m");
        assert_eq!(client.registered_by_profile_id, *admin);
        assert!(client.revoked_at.is_none());

        let link = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_profile_auth_links \
              WHERE auth_provider = 'auth0-m2m' AND auth_provider_user_id = 'acme-agent'",
        )
        .fetch_one(&pool)
        .await
        .expect("count link");
        assert_eq!(link, Some(1));

        let emitters = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_entities WHERE profile_id = $1",
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count emitters");
        assert_eq!(emitters, Some(4), "one emitter per Surface::ALL variant");
    }

    /// D14: the trigger auto-joins only while access_mode='open'. provision must not
    /// depend on it, or every machine 403s the day the instance flips to invite_only.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_enrolls_the_agent_in_the_gating_team(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        // Mirror prod's real invite_only shape: a configured gating team. A fresh test DB
        // seeds gating_team_slug NULL, and `update_system_settings` rejects invite_only with
        // no slug precisely because it would lock everyone out — so a bare access_mode flip is
        // not a state the product ever reaches.
        sqlx::query!(
            "UPDATE kb_system_settings SET access_mode = 'invite_only', gating_team_slug = 'temper-system'"
        )
        .execute(&pool)
        .await
        .expect("flip to invite_only");

        let client = svc::provision(&pool, admin, &req("gated-agent"))
            .await
            .expect("provision");

        let has_access = sqlx::query_scalar!("SELECT has_system_access($1)", client.profile_id)
            .fetch_one(&pool)
            .await
            .expect("has_system_access");
        assert_eq!(
            has_access,
            Some(true),
            "a provisioned machine must pass the system gate under invite_only (D14)"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_applies_explicit_team_and_cogmap_reach(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let team_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_teams (id, slug, name) VALUES ($1, 'acme', 'Acme')",
            team_id,
        )
        .execute(&pool)
        .await
        .expect("seed team");

        // kb_cogmaps requires a telos_resource_id; seed a throwaway resource for it.
        let telos_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, 'acme-telos', '')",
            telos_id,
        )
        .execute(&pool)
        .await
        .expect("seed telos resource");
        let cogmap_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_cogmaps (id, name, telos_resource_id) VALUES ($1, 'Acme Map', $2)",
            cogmap_id,
            telos_id,
        )
        .execute(&pool)
        .await
        .expect("seed cogmap");

        let request = ProvisionMachineRequest {
            client_id: "reach-agent".to_string(),
            label: "steward".to_string(),
            owner_team_id: Some(team_id),
            teams: vec![TeamSpec {
                team_id,
                role: "member".to_string(),
            }],
            grants: vec![GrantSpec {
                cogmap_id,
                can_write: true,
            }],
        };
        let client = svc::provision(&pool, admin, &request)
            .await
            .expect("provision");

        assert_eq!(client.team_id, Some(team_id), "owner is recorded");

        let member = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
            team_id,
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count membership");
        assert_eq!(member, Some(1));

        let grant = sqlx::query!(
            "SELECT can_read, can_write, can_grant, can_delete FROM kb_access_grants \
              WHERE subject_table = 'kb_cogmaps' AND subject_id = $1 \
                AND principal_table = 'kb_profiles' AND principal_id = $2",
            cogmap_id,
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("grant row");
        assert!(
            grant.can_read && grant.can_write,
            "write implies read (DB coherence CHECK)"
        );
        // D6: a machine never receives re-delegation or deletion, regardless of who minted it.
        assert!(
            !grant.can_grant,
            "a machine grant must never carry can_grant (D6)"
        );
        assert!(
            !grant.can_delete,
            "a machine grant must never carry can_delete"
        );
    }

    /// The regression test for the silent identity fork (D8).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rebind_preserves_the_agent_profile_and_revokes_the_old_client(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let old = svc::provision(&pool, admin, &req("old-client"))
            .await
            .expect("provision");

        let new = svc::rebind(
            &pool,
            admin,
            &RebindMachineRequest {
                client_id: "new-client".to_string(),
                from_machine_client_id: old.id,
                label: "steward (rotated)".to_string(),
                keep_old_active: false,
            },
        )
        .await
        .expect("rebind");

        assert_eq!(
            new.profile_id, old.profile_id,
            "a rotated application must not fork the machine's identity"
        );

        let old_row = crate::services::machine_client_service::get(&pool, old.id)
            .await
            .expect("old row");
        assert!(
            old_row.revoked_at.is_some(),
            "the old client is revoked in the same transaction"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rebind_with_keep_old_active_leaves_an_overlap_window(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let old = svc::provision(&pool, admin, &req("overlap-old"))
            .await
            .expect("provision");

        svc::rebind(
            &pool,
            admin,
            &RebindMachineRequest {
                client_id: "overlap-new".to_string(),
                from_machine_client_id: old.id,
                label: "steward".to_string(),
                keep_old_active: true,
            },
        )
        .await
        .expect("rebind");

        let old_row = crate::services::machine_client_service::get(&pool, old.id)
            .await
            .expect("old row");
        assert!(
            old_row.revoked_at.is_none(),
            "--no-revoke-old keeps both credentials live"
        );
    }

    /// Rebind is system-admin-only (B2). A team owner — who may provision/issue/revoke/rotate
    /// their own team's machines — must NOT be able to rebind, because rebind inherits the old
    /// profile's full reach, which the owner's authority cannot bound.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rebind_is_refused_for_a_non_admin_team_owner(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        // A team, and Alice who owns it but is not a system admin.
        let team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ('acme', 'Acme') RETURNING id"
        )
        .fetch_one(&pool)
        .await
        .expect("team");
        let alice = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, 'alice', 'Alice')",
            alice,
        )
        .execute(&pool)
        .await
        .expect("alice");
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner'::team_role)",
            team,
            alice,
        )
        .execute(&pool)
        .await
        .expect("alice owns acme");

        // Admin provisions a machine owned by Alice's team.
        let mut provision_req = req("acme-agent-rb");
        provision_req.owner_team_id = Some(team);
        let old = svc::provision(&pool, admin, &provision_req)
            .await
            .expect("provision");

        // Alice owns the machine's team — she can revoke it (tested elsewhere) — but she may NOT
        // rebind it onto a client_id she controls and inherit its identity.
        let err = svc::rebind(
            &pool,
            ProfileId::from(alice),
            &RebindMachineRequest {
                client_id: "alice-controls-this".to_string(),
                from_machine_client_id: old.id,
                label: "hijack".to_string(),
                keep_old_active: false,
            },
        )
        .await
        .expect_err("a non-admin team owner must not rebind");
        assert!(
            matches!(err, crate::error::ApiError::Forbidden),
            "got {err:?}"
        );
    }

    /// A revoked credential is dead: rebind must refuse to resurrect it (revoke leaves the
    /// profile's grants/memberships live, so a rebind would silently revive that reach).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rebind_refuses_a_revoked_source(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let old = svc::provision(&pool, admin, &req("to-be-revoked"))
            .await
            .expect("provision");

        crate::services::machine_client_service::revoke(&pool, old.id, admin)
            .await
            .expect("revoke");

        let err = svc::rebind(
            &pool,
            admin,
            &RebindMachineRequest {
                client_id: "resurrected".to_string(),
                from_machine_client_id: old.id,
                label: "back from the dead".to_string(),
                keep_old_active: false,
            },
        )
        .await
        .expect_err("a revoked source must not be rebindable");
        assert!(
            matches!(err, crate::error::ApiError::BadRequest(_)),
            "got {err:?}"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn issue_mints_a_temper_credential_with_a_stored_hash(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let cred = svc::issue(
            &pool,
            admin,
            &IssueMachineRequest {
                label: "sidekiq".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
        )
        .await
        .expect("issue");

        assert!(
            cred.client.client_id.starts_with("tmpr_"),
            "temper mints the id"
        );
        assert_eq!(cred.client.issuer, "temper");
        assert!(!cred.client_secret.is_empty(), "plaintext returned once");
        assert_eq!(cred.client.registered_by_profile_id, *admin);

        // The stored hash is the SHA-256 of the returned plaintext; the plaintext itself is
        // never persisted.
        let stored: Option<String> = sqlx::query_scalar!(
            "SELECT secret_hash FROM kb_machine_clients WHERE id = $1",
            cred.client.id,
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            stored.as_deref(),
            Some(crate::auth::secret::sha256_hex(&cred.client_secret).as_str()),
        );

        // The auth link uses the machine-principal namespace, NOT 'temper' (D5).
        let link = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_profile_auth_links \
              WHERE auth_provider = 'auth0-m2m' AND auth_provider_user_id = $1",
            cred.client.client_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count link");
        assert_eq!(link, Some(1));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provisioning_a_duplicate_client_id_is_a_conflict(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        svc::provision(&pool, admin, &req("dupe"))
            .await
            .expect("first");
        let err = svc::provision(&pool, admin, &req("dupe"))
            .await
            .expect_err("second must fail");
        assert!(
            matches!(err, crate::error::ApiError::Conflict(_)),
            "got {err:?}"
        );
    }
}
