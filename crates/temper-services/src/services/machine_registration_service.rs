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
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{MachineClient, ProvisionMachineRequest, RebindMachineRequest};

use crate::error::{ApiError, ApiResult};
use crate::services::access_service::{self, insert_grant, InsertGrantParams};
use crate::services::machine_authz::{self, AuthorizedReach};
use crate::services::machine_client_service;
use crate::services::profile_service;

/// Enroll `profile_id` in the configured gating team as `watcher`.
///
/// D14: `trg_sync_system_membership` auto-joins new profiles ONLY while
/// `access_mode = 'open'`, because `has_system_access` short-circuits true under that
/// mode. Under `invite_only` it enrolls nothing, and an unenrolled machine authenticates
/// and then 403s at `require_system_access`. So we enroll explicitly, exactly as
/// `access_service::review_request` does for an approved human. Never depend on the
/// trigger: its behavior is a function of a setting that is about to change.
async fn enroll_in_gating_team(conn: &mut sqlx::PgConnection, profile_id: Uuid) -> ApiResult<()> {
    let slug = sqlx::query_scalar!("SELECT gating_team_slug FROM kb_system_settings LIMIT 1")
        .fetch_optional(&mut *conn)
        .await?
        .flatten();

    let Some(slug) = slug else {
        // No gating team configured ⇒ nothing to enroll into. `update_system_settings`
        // already rejects `invite_only` with an empty slug, so this is the open-mode case.
        return Ok(());
    };

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
    enroll_in_gating_team(&mut tx, profile_id).await?;
    apply_reach(&mut tx, caller, profile_id, reach).await?;

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
    enroll_in_gating_team(&mut tx, profile_id).await?;
    apply_reach(&mut tx, caller, profile_id, reach).await?;

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
