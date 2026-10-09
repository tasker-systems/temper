//! Transactional registration of machine principals.
//!
//! `provision` is the inversion (D3): it creates the agent profile, its auth link, its
//! emitter entities, its explicit reach, and the `kb_machine_clients` row — all in ONE
//! transaction, ahead of the machine's first call.
//!
//! **A machine joins no team it was not explicitly given.** Registration used to enroll every
//! machine in the gating team as `watcher` (D14). That enrollment was retired (ruled 2026-10-09):
//! under D11 gating-team membership confers no system access, and a machine holds membership only
//! where someone with authority over the team chose it — the explicit `teams` reach, bounded by
//! `machine_authz`. Its personal team still sits under `temper-system`, so what reaches every
//! member of the root through ancestry reaches the machine too.
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

use crate::auth::{AuthenticatedProfile, SystemAdmin};
use crate::error::{ApiError, ApiResult};
use crate::services::access_service::{insert_grant, InsertGrantParams};
use crate::services::machine_authz::{self, AuthorizedReach};
use crate::services::machine_client_service;
use crate::services::profile_service;

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
            // Per-ROW warrant: the subject is this grant's own cogmap, read from the sealed item
            // rather than named again here.
            &crate::authz::GrantWarrant::MachineReach(grant),
            &InsertGrantParams {
                principal_table: "kb_profiles".to_string(),
                principal_id: profile_id,
                // Write implies read — the DB's coherence CHECK enforces it anyway.
                can_read: true,
                can_write: grant.can_write(),
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
    authed: &AuthenticatedProfile,
    req: &ProvisionMachineRequest,
) -> ApiResult<MachineClient> {
    let caller = ProfileId::from(authed.profile().id);
    // Auth before writes: a rejected registration must leave the DB completely unchanged —
    // no orphaned agent profile, no partial enrollment. Resolving before the transaction is
    // what makes that assertable.
    let reach = machine_authz::authorize_registration(
        pool,
        authed,
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

    // D11 — every mint door births Denied; even a machine minted by an admin gets no access.
    // Containment is retired, not relocated: a minter who cannot confer access is moot when minting
    // never confers any. Raw principal_standing_apply on the transaction, NOT
    // standing_service::provision — that takes &PgPool and would write outside this tx, risking an
    // orphaned standing row if the registration rolls back.
    //
    // `caller` is the actor, and passing NULL here was a real attribution hole rather than a
    // stylistic one: the committer resolves its emitter through `COALESCE(p_actor, p_profile)`
    // (20260720000030), so a NULL actor makes the emitter the SUBJECT'S OWN entity. The ledger then
    // could not say who registered a machine, and the machine's own actor-axis read returned a
    // `provision` it did not perform. The registrar is already recorded one statement above, in
    // `kb_machine_clients.registered_by_profile_id` — the ledger now says what the row says.
    sqlx::query_scalar!(
        "SELECT principal_standing_apply($1,'provision','denied',$2,'machine registration')",
        profile_id,
        *caller,
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    machine_client_service::get(pool, id).await
}

/// Issue a temper-minted machine credential (Phase B1). temper generates the `client_id` and
/// the secret; the SHA-256 hex of the secret is stored, the plaintext is returned once. Creates
/// the agent profile, auth link, emitters, and reach — all in one
/// transaction, exactly like `provision`, but with `issuer='temper'` and a `secret_hash`.
pub async fn issue(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
    req: &temper_core::types::machine::IssueMachineRequest,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    let caller = ProfileId::from(authed.profile().id);
    // Auth before writes — same reasoning as `provision`: nothing is minted, and no profile
    // is created, unless the caller may confer this reach.
    let reach = machine_authz::authorize_registration(
        pool,
        authed,
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

    // D11 — the temper-minted machine door births Denied too. `issue` is `provision`'s structural
    // twin (a second mint door), so it carries the same born-Denied standing; leaving it unwired is
    // exactly the carelessly-added door the whole-surface property guards against. It carries the
    // actor for the same reason too — see the note on `provision`'s call.
    sqlx::query_scalar!(
        "SELECT principal_standing_apply($1,'provision','denied',$2,'machine issue')",
        profile_id,
        *caller,
    )
    .fetch_one(&mut *tx)
    .await?;

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
    admin: &SystemAdmin,
    req: &RebindMachineRequest,
) -> ApiResult<MachineClient> {
    // Auth before writes. Admin-only (see the fn doc): team ownership cannot bound the reach a rebind
    // inherits — which is why this takes a `&SystemAdmin` proof, NOT `machine_authz`. The proof itself
    // IS the check (admin-authz enclosure, spec §3); do not widen it back to a scoped gate.
    let old = machine_client_service::get(pool, req.from_machine_client_id).await?;

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
        *admin.actor(),
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
            *admin.actor(),
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
    /// itself now resolves the caller's authority. Under D11 an admin is a `kb_principal_governance`
    /// grant (`is_system_admin`) with an `approved` `kb_principal_standing` (`has_system_access`),
    /// not gating-team ownership — so the profile is seeded with both.
    ///
    /// The admin is also an owner of the gating team, the shape that used to enroll every machine it
    /// minted; registration must not, so the fixture keeps the minter inside the team. Being a
    /// gating-team owner confers no admin-ness. `temper-system` already exists in a migrated
    /// database (the L0 kernel migration creates it), so the team write is an upsert.
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

        // What confers admin-ness now: approved standing (front door) + a governance grant.
        crate::test_support::approved_admin(pool, id).await;

        ProfileId::from(id)
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

        let client = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("acme-agent"),
        )
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

    /// Registration enrolls the machine in no team it was not given, the gating team included,
    /// even when the minter is inside it. Retired D14 enrollment; the machine stays born Denied.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_does_not_enroll_the_machine_in_the_gating_team(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let client = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("gated-agent"),
        )
        .await
        .expect("provision");

        assert_eq!(gating_memberships(&pool, client.profile_id).await, 0);
        let has_access = sqlx::query_scalar!("SELECT has_system_access($1)", client.profile_id)
            .fetch_one(&pool)
            .await
            .expect("has_system_access");
        assert_eq!(
            has_access,
            Some(false),
            "a freshly provisioned machine is born Denied (D11)"
        );
    }

    /// The same on `issue`: the mint path enrolls nothing either.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn issue_does_not_enroll_the_machine_in_the_gating_team(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let cred = svc::issue(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &IssueMachineRequest {
                label: "sidekiq".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
        )
        .await
        .expect("issue");

        assert_eq!(gating_memberships(&pool, cred.client.profile_id).await, 0);
    }

    /// Provision a machine and approve its standing through the one committer, which runs the
    /// auto-join enrollment arm. Returns the machine's profile.
    async fn approved_machine(pool: &PgPool, admin: ProfileId, client_id: &str) -> Uuid {
        let client = svc::provision(
            pool,
            &crate::test_support::authenticated_profile_for(pool, admin.uuid()).await,
            &req(client_id),
        )
        .await
        .expect("provision");
        sqlx::query_scalar!(
            "SELECT principal_standing_apply($1, 'approve', 'approved', $2, NULL)",
            client.profile_id,
            *admin,
        )
        .fetch_one(pool)
        .await
        .expect("approve the machine");
        client.profile_id
    }

    /// Approval makes a machine eligible by standing, and auto-join (`temper-system` carries
    /// `auto_join_role = 'watcher'`) must still not enroll it.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_approved_machine_is_not_auto_joined(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "approved-agent").await;
        let has_access = sqlx::query_scalar!("SELECT has_system_access($1)", machine)
            .fetch_one(&pool)
            .await
            .expect("has_system_access");
        assert_eq!(has_access, Some(true), "precondition: eligible by standing");
        assert_eq!(gating_memberships(&pool, machine).await, 0);
    }

    /// Creating an auto-join team backfills every eligible profile, and a machine is not one.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn backfilling_an_auto_join_team_skips_machines(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "backfill-agent").await;
        let pool_team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name, auto_join_role) \
             VALUES ('everyone', 'Everyone', 'member'::team_role) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("auto-join team");
        sqlx::query!("SELECT backfill_auto_join_team($1)", pool_team)
            .execute(&pool)
            .await
            .expect("backfill");

        let joined = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
            pool_team,
            machine,
        )
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(joined, Some(0));
        let admin_joined = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
            pool_team,
            *admin,
        )
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(
            admin_joined,
            Some(1),
            "the backfill still enrolls an eligible human"
        );
    }

    /// The operator repair path converges auto-join teams to approved humans only. The team is
    /// created after approval and not backfilled, so reconcile is the only thing that could add
    /// the machine.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn auto_join_reconcile_skips_machines(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "reconcile-agent").await;
        let pool_team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name, auto_join_role) \
             VALUES ('everyone', 'Everyone', 'member'::team_role) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("auto-join team");

        let added: Vec<Option<String>> =
            sqlx::query_scalar!("SELECT team_slug FROM auto_join_reconcile()")
                .fetch_all(&pool)
                .await
                .expect("reconcile");
        assert!(
            added.iter().flatten().any(|t| t == "everyone"),
            "precondition: reconcile still adds the eligible human; added {added:?}"
        );
        let joined = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
            pool_team,
            machine,
        )
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(joined, Some(0));
    }

    /// Approving a machine's join request admits it by standing and enrolls it nowhere: the
    /// approval's gating-team `watcher` row is for people.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn approving_a_machines_join_request_does_not_enroll_it(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let client = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("requesting-agent"),
        )
        .await
        .expect("provision");
        let request = access_service::create_join_request(
            &pool,
            access_service::CreateJoinRequestParams {
                profile_id: ProfileId::from(client.profile_id),
                message: None,
                source: "test".to_string(),
                accepted_terms_version: None,
            },
            None,
        )
        .await
        .expect("a born-Denied machine may request");
        access_service::review_request(
            &pool,
            &crate::test_support::system_admin_proof_for(&pool, admin.uuid()).await,
            access_service::ReviewRequestParams {
                request_id: request.id,
                decision: temper_core::types::access_gate::JoinRequestStatus::Approved,
                decision_note: None,
            },
        )
        .await
        .expect("approve");

        assert_eq!(gating_memberships(&pool, client.profile_id).await, 0);
    }

    /// Pin, not a bite: without a direct row, a machine still reaches the root through its
    /// personal team, the path every content gate walks (`profile_reachable_teams`).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_machine_reaches_the_root_through_its_personal_team(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "reach-agent").await;
        let reaches = sqlx::query_scalar!(
            r#"SELECT EXISTS(
                 SELECT 1 FROM profile_reachable_teams($1) r
                   JOIN kb_teams t ON t.id = r.team_id
                  WHERE t.slug = 'temper-system') AS "r!: bool""#,
            machine,
        )
        .fetch_one(&pool)
        .await
        .expect("reach");
        assert!(reaches);
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
        let client = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &request,
        )
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
        let old = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("old-client"),
        )
        .await
        .expect("provision");

        let proof = crate::test_support::system_admin_proof_for(&pool, *admin).await;
        let new = svc::rebind(
            &pool,
            &proof,
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
        let old = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("overlap-old"),
        )
        .await
        .expect("provision");

        let proof = crate::test_support::system_admin_proof_for(&pool, *admin).await;
        svc::rebind(
            &pool,
            &proof,
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
        let old = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &provision_req,
        )
        .await
        .expect("provision");

        // Alice owns the machine's team — she can revoke it (tested elsewhere) — but she may NOT
        // rebind it onto a client_id she controls and inherit its identity. Post-enclosure the bar is
        // structural: rebind requires a `&SystemAdmin`, and a non-admin cannot mint one. The refusal
        // now happens at the proof gate, before rebind is even reachable.
        let alice_authed = crate::test_support::authenticated_profile_for(&pool, alice).await;
        let err = crate::auth::require_system_admin(&pool, &alice_authed)
            .await
            .expect_err("a non-admin team owner cannot mint an admin proof");
        assert!(
            matches!(err, crate::error::ApiError::Forbidden),
            "got {err:?}"
        );

        // And the machine is untouched — the refusal happened before any rebind write.
        let still = crate::services::machine_client_service::get(&pool, old.id)
            .await
            .expect("old row");
        assert!(
            still.revoked_at.is_none(),
            "the refused caller changed nothing"
        );
    }

    /// A revoked credential is dead: rebind must refuse to resurrect it (revoke leaves the
    /// profile's grants/memberships live, so a rebind would silently revive that reach).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rebind_refuses_a_revoked_source(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let old = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("to-be-revoked"),
        )
        .await
        .expect("provision");

        crate::services::machine_client_service::revoke(
            &pool,
            old.id,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
        )
        .await
        .expect("revoke");

        let proof = crate::test_support::system_admin_proof_for(&pool, *admin).await;
        let err = svc::rebind(
            &pool,
            &proof,
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
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
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
        svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("dupe"),
        )
        .await
        .expect("first");
        let err = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("dupe"),
        )
        .await
        .expect_err("second must fail");
        assert!(
            matches!(err, crate::error::ApiError::Conflict(_)),
            "got {err:?}"
        );
    }

    /// The mint doors attribute their registrar ON THE LEDGER, not only on the row.
    ///
    /// Both doors used to pass `NULL` as `principal_standing_apply`'s actor. That is not a
    /// stylistic omission: the committer resolves its emitter through
    /// `COALESCE(p_actor, p_profile)` (`migrations/20260720000030`), so a NULL actor made the
    /// emitter the SUBJECT'S OWN entity. Two consequences, both invisible while the standing types
    /// were readable through no door at all:
    ///   - the ledger could not say who registered a machine, though
    ///     `kb_machine_clients.registered_by_profile_id` had recorded it all along;
    ///   - the machine's own actor-axis read returned a `provision` it did not perform, since it
    ///     was its own emitter.
    ///
    /// Asserted through `admin_ledger_service` rather than against `kb_events` directly, because
    /// the actor axis is a two-hop join (`kb_events` → `kb_entities` → `kb_profiles`) and it is
    /// that resolution — not the payload field alone — that a reader actually gets.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn both_mint_doors_name_their_registrar_on_the_ledger(pool: PgPool) {
        use crate::services::admin_ledger_service;
        use temper_substrate::payloads::{AnchorTable, RefTarget};

        let admin = seed_admin(&pool).await;

        let provisioned = svc::provision(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &req("attributed-agent"),
        )
        .await
        .expect("provision");
        let issued = svc::issue(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            &IssueMachineRequest {
                label: "attributed-issue".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
        )
        .await
        .expect("issue");

        for (door, machine_profile) in [
            ("provision", provisioned.profile_id),
            ("issue", issued.client.profile_id),
        ] {
            // The subject axis: an operator asking "who registered this machine?" gets an answer.
            let entries = admin_ledger_service::list_by_subject(
                &pool,
                &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
                RefTarget {
                    kind: AnchorTable::Profiles,
                    id: machine_profile,
                },
                50,
                0,
            )
            .await
            .unwrap_or_else(|e| {
                panic!("{door}: admin reads the machine's standing history: {e:?}")
            });

            let birth = entries
                .iter()
                .find(|e| {
                    e.event_type == "principal_standing_changed" && e.payload["act"] == "provision"
                })
                .unwrap_or_else(|| panic!("{door}: the machine's provision must be on the ledger"));

            assert_eq!(
                birth.actor_profile_id, *admin,
                "{door}: the ledger must name the registrar, as \
                 kb_machine_clients.registered_by_profile_id already does",
            );
            assert_eq!(
                birth.payload["actor"],
                admin.uuid().to_string(),
                "{door}: the payload must carry the actor too — jsonb_strip_nulls drops the key \
                 entirely when NULL is passed, which is how the hole read as an absent field \
                 rather than as a wrong one",
            );
        }

        // The other half: the registrar's own history now carries both mints, and the machine's
        // does not carry an act it never performed.
        let own = admin_ledger_service::list_by_actor(
            &pool,
            &crate::test_support::authenticated_profile_for(&pool, admin.uuid()).await,
            admin,
            100,
            0,
        )
        .await
        .expect("the registrar reads their own acts");
        let provisions = own
            .iter()
            .filter(|e| {
                e.event_type == "principal_standing_changed" && e.payload["act"] == "provision"
            })
            .count();
        assert_eq!(
            provisions, 2,
            "both mint doors must land on the registrar's own actor axis, not the machine's",
        );
    }
}
