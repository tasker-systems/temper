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
//! member of the root through ancestry reaches the machine too. It holds that team as `member`,
//! never `owner`: a machine never governs (`20261019100000_machines_never_govern.sql`).
//!
//! Authorization happens HERE, not in the handler (B2 D3): `provision` and `issue` resolve the
//! caller's authority through `machine_authz` before opening the transaction, so a rejected
//! registration leaves the database completely unchanged. They record the authorized caller as
//! `registered_by_profile_id`. Ahead of that, the instance's mode decides which doors are open at
//! all (`crate::auth::machine_door`): under an external IdP `provision` is held to a system admin,
//! and under temper's own AS it is refused. `rebind` is **system-admin-only** in every mode (it
//! transplants an existing profile's reach, which team ownership cannot bound — see its doc).

use sqlx::PgPool;
use temper_substrate::ids::EntityId;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{MachineClient, ProvisionMachineRequest, RebindMachineRequest};

use crate::auth::machine_door::{self, DoorPolicy, MachineDoor};
use crate::auth::{HumanPrincipal, SystemAdmin};
use crate::auth_config::AuthMode;
use crate::authz::team_membership;
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
/// Team memberships are recorded acts (plan C3): each team is locked in ascending id order
/// (the multi-writer lock order) and the row goes through `from_reach` +
/// `membership_write`, so a reach lands as a `machine_reach` event naming the registrar as
/// actor. Under each team lock the registrar's membership authority is resolved again on the
/// transaction and `from_reach` re-applies the bar the pool check applied (task `01a14592-3f83`),
/// so the decision reads current state, as every member act's does (plan P1). The role ceiling
/// was enforced at mint (D4b) and again by `from_reach`; the machine row is inserted after the
/// reach, and the database's machine-cap trigger holds the ceiling then. The grant half keeps
/// `insert_grant` — its own sealed `GrantWarrant` authority, authored by the registrar's resolved
/// emitter.
async fn apply_reach(
    conn: &mut sqlx::PgConnection,
    profile_id: Uuid,
    reach: AuthorizedReach<'_>,
    // `Some` iff `reach` carries at least one grant (the caller resolves one only when there is a
    // grant to author). The membership events emit from it when present; when absent,
    // `membership_write` resolves the registrar's own `<handle>@web` entity. Either way the SQL
    // writer ties the emitter to the recorded actor, which is this registrar.
    emitter: Option<EntityId>,
) -> ApiResult<()> {
    let registrar = reach.registrar();
    let mut teams: Vec<_> = reach.teams().iter().collect();
    teams.sort_by_key(|team| team.team_id);
    for team in teams {
        let role = machine_authz::parse_team_role(&team.role)?;
        let target =
            team_membership::locked(&mut *conn, team.team_id, ProfileId::from(profile_id)).await?;
        let resolved =
            team_membership::MembershipAuthority::resolve(&mut *conn, registrar, team.team_id)
                .await?;
        let change = team_membership::AuthorizedMembershipChange::from_reach(
            &reach, resolved, &target, role,
        )?;
        team_membership::membership_write(&mut *conn, emitter, change).await?;
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
                granted_by_profile_id: *registrar.profile_id(),
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

/// Refuse a caller-supplied client id in a namespace temper mints into
/// ([`MINTED_CLIENT_ID_PREFIXES`](crate::auth::secret::MINTED_CLIENT_ID_PREFIXES), which says why).
/// `provision` and `rebind` are the two doors that take a client id from the caller; `issue`
/// mints its own.
fn refuse_minted_namespace(client_id: &str) -> ApiResult<()> {
    match crate::auth::secret::MINTED_CLIENT_ID_PREFIXES
        .iter()
        .find(|prefix| client_id.starts_with(*prefix))
    {
        Some(prefix) => Err(ApiError::BadRequest(format!(
            "client id '{client_id}' is in the '{prefix}' namespace, which temper mints itself; \
             register the client id your identity provider issued"
        ))),
        None => Ok(()),
    }
}

/// Register a new machine principal, creating its agent profile. One transaction.
///
/// What `mode` permits comes first ([`machine_door::machine_door_policy`]). Under an external IdP
/// only a system admin may register a client id: only the IdP's administrator can create a client
/// worth registering, and a team owner who registered one they did not create would receive its
/// tokens. On an instance that runs temper's own authorization server an externally issued client
/// id could never authenticate, so the door is closed.
pub async fn provision(
    pool: &PgPool,
    authed: &HumanPrincipal,
    req: &ProvisionMachineRequest,
    mode: AuthMode,
) -> ApiResult<MachineClient> {
    match machine_door::machine_door_policy(mode, MachineDoor::Provision) {
        DoorPolicy::Refused(why) => return Err(ApiError::BadRequest(why.to_string())),
        DoorPolicy::AdminOnly => match crate::auth::require_system_admin(pool, authed).await {
            Ok(_) => {}
            // Only a genuine non-admin is answered with the reason; a failed read stays a 5xx.
            Err(ApiError::Forbidden) => {
                return Err(ApiError::ForbiddenDetail(
                    "registering an externally issued client id requires a system admin on this \
                     instance"
                        .to_string(),
                ));
            }
            Err(other) => return Err(other),
        },
        DoorPolicy::Allowed | DoorPolicy::Dormant => {}
    }

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
    refuse_minted_namespace(&req.client_id)?;

    // Resolve the caller's emitter BEFORE the transaction — matching the auth-before-tx pattern above,
    // and avoiding a nested pool acquire while `tx` is open — when there is a grant to author. A
    // pure team-membership reach skips this; its `machine_reach` events resolve the registrar's
    // `<handle>@web` entity inside the transaction instead (`membership_write`), so either way every
    // recorded act emits as the registrar.
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
    apply_reach(&mut tx, profile_id, reach, emitter).await?;

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
///
/// Under an external IdP the credential is minted **dormant**: no token for it can be issued
/// until the instance runs temper's own authorization server, and the response says so. That is
/// deliberate, so an operator can stage credentials before moving an instance to its own AS.
pub async fn issue(
    pool: &PgPool,
    authed: &HumanPrincipal,
    req: &temper_core::types::machine::IssueMachineRequest,
    mode: AuthMode,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    let dormant = match machine_door::machine_door_policy(mode, MachineDoor::Issue) {
        DoorPolicy::Refused(why) => return Err(ApiError::BadRequest(why.to_string())),
        DoorPolicy::Dormant => true,
        DoorPolicy::Allowed | DoorPolicy::AdminOnly => false,
    };
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

    // Resolve the caller's emitter BEFORE the transaction (see `provision`) when a grant is to be
    // authored; a pure team-membership reach resolves it inside the transaction (`membership_write`).
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
    apply_reach(&mut tx, profile_id, reach, emitter).await?;

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
        dormant,
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
///
/// It writes an externally issued client id, so it is refused where `mode` could never
/// authenticate one ([`machine_door::machine_door_policy`]), and for a temper-issued source row in
/// any mode: that machine's credential is rotated, not rebound to an external one.
pub async fn rebind(
    pool: &PgPool,
    admin: &SystemAdmin,
    req: &RebindMachineRequest,
    mode: AuthMode,
) -> ApiResult<MachineClient> {
    if let DoorPolicy::Refused(why) = machine_door::machine_door_policy(mode, MachineDoor::Rebind) {
        return Err(ApiError::BadRequest(why.to_string()));
    }

    // Auth before writes. Admin-only (see the fn doc): team ownership cannot bound the reach a rebind
    // inherits — which is why this takes a `&SystemAdmin` proof, NOT `machine_authz`. The proof itself
    // IS the check (admin-authz enclosure, spec §3); do not widen it back to a scoped gate.
    let old = machine_client_service::get(pool, req.from_machine_client_id).await?;

    // The new client id is the caller's choice, so it may not sit in a namespace temper mints.
    refuse_minted_namespace(&req.client_id)?;

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

    // A temper-issued machine has a secret to rotate. Rebinding it would move its profile onto an
    // external client id, and on an instance running its own AS that machine stops authenticating.
    // After the revoked check, so a revoked source is told to issue anew rather than to rotate.
    if old.issuer == machine_door::TEMPER_ISSUER {
        return Err(ApiError::BadRequest(format!(
            "machine client '{}' was issued by temper; rotate its secret with \
             `temper admin machine rotate-secret` instead",
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("acme-agent"),
            crate::auth_config::AuthMode::ExternalIdp,
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("gated-agent"),
            crate::auth_config::AuthMode::ExternalIdp,
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &IssueMachineRequest {
                label: "sidekiq".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
            crate::auth_config::AuthMode::ExternalIdp,
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
            &crate::test_support::human_principal_for(pool, admin.uuid()).await,
            &req(client_id),
            crate::auth_config::AuthMode::ExternalIdp,
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

    /// Approval makes a machine eligible by standing, and the committer's auto-join arm must still
    /// not enroll it. The pool is created before approval and is not the gating team, so only that
    /// arm could put the machine there.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_approved_machine_is_not_auto_joined(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let pool_team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name, auto_join_role) \
             VALUES ('everyone', 'Everyone', 'member'::team_role) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("auto-join team");
        let machine = approved_machine(&pool, admin, "approved-agent").await;
        let has_access = sqlx::query_scalar!("SELECT has_system_access($1)", machine)
            .fetch_one(&pool)
            .await
            .expect("has_system_access");
        assert_eq!(has_access, Some(true), "precondition: eligible by standing");
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
    ///
    /// A machine cannot file a join request (`create_join_request` takes a person), so the request
    /// is seeded as the standing transition and row that filing writes — the shape a request filed
    /// before that line still has.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn approving_a_machines_join_request_does_not_enroll_it(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let client = svc::provision(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("requesting-agent"),
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("provision");
        sqlx::query_scalar!(
            "SELECT principal_standing_apply($1, 'request', 'requested', $1, NULL)",
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("a born-Denied machine moves to requested");
        let request_id: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_join_requests \
                 (id, team_id, requesting_profile_id, status, source, created, updated) \
             SELECT $1, t.id, $2, 'pending', 'test', now(), now() \
               FROM kb_teams t JOIN kb_system_settings s ON t.slug = s.gating_team_slug \
             RETURNING id",
            Uuid::now_v7(),
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("the machine's pending request on the gating team");
        // Take `temper-system` out of auto-join, so the approval's own `watcher` insert is the only
        // thing that could put the machine in the gating team.
        sqlx::query!("UPDATE kb_teams SET auto_join_role = NULL WHERE slug = 'temper-system'")
            .execute(&pool)
            .await
            .expect("no auto-join on the gating team");
        access_service::review_request(
            &pool,
            &crate::test_support::system_admin_proof_for(&pool, admin.uuid()).await,
            access_service::ReviewRequestParams {
                request_id,
                decision: temper_core::types::access_gate::JoinRequestStatus::Approved,
                decision_note: None,
            },
        )
        .await
        .expect("approve");

        assert_eq!(gating_memberships(&pool, client.profile_id).await, 0);
        // The skip records nothing (C8): the machine skip is not a silent membership write.
        let events: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'team_membership_changed'",
        )
        .fetch_one(&pool)
        .await
        .expect("count membership events")
        .unwrap_or(0);
        assert_eq!(events, 0, "a machine approval records no membership change");
    }

    /// C3: a provisioned team reach is a recorded act — exactly one `machine_reach` event with
    /// the registrar as actor, the offered role as written, no previous role (an add).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_records_one_machine_reach_event(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team: Uuid =
            sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
                .fetch_one(&pool)
                .await
                .expect("gating team");
        let request = ProvisionMachineRequest {
            client_id: "reach-event-agent".to_string(),
            label: "steward".to_string(),
            owner_team_id: Some(team),
            teams: vec![TeamSpec {
                team_id: team,
                role: "member".to_string(),
            }],
            grants: vec![],
        };
        let client = svc::provision(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &request,
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("provision");

        let events: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT e.payload FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'team_membership_changed' ORDER BY e.id",
        )
        .fetch_all(&pool)
        .await
        .expect("membership events");
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0]["via"], "machine_reach", "{events:?}");
        assert_eq!(events[0]["actor"], serde_json::json!(admin.uuid()));
        assert_eq!(
            events[0]["profile_id"],
            serde_json::json!(client.profile_id)
        );
        assert_eq!(events[0]["role"], "member", "{events:?}");
        assert!(events[0].get("previous_role").is_none(), "{events:?}");
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &request,
            crate::auth_config::AuthMode::ExternalIdp,
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("old-client"),
            crate::auth_config::AuthMode::ExternalIdp,
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
            crate::auth_config::AuthMode::ExternalIdp,
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("overlap-old"),
            crate::auth_config::AuthMode::ExternalIdp,
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
            crate::auth_config::AuthMode::ExternalIdp,
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

    /// Rebind is system-admin-only (B2). A team owner — who may issue/revoke/rotate
    /// their own team's machines — must NOT be able to rebind, because rebind inherits the old
    /// profile's full reach, which the owner's authority cannot bound.
    /// Standing belongs to the profile, and a rebind overlap window puts two live credentials on one.
    /// An admin closing the window by revoking the old credential must not revoke standing, or the
    /// new credential, the one being kept, stops being admitted. Revoking the last live credential
    /// still revokes standing (D17).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn closing_an_overlap_window_keeps_the_surviving_credential_admitted(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let profile = approved_machine(&pool, admin, "window-old").await;
        let old: Uuid =
            sqlx::query_scalar("SELECT id FROM kb_machine_clients WHERE client_id = 'window-old'")
                .fetch_one(&pool)
                .await
                .expect("old row");

        let proof = crate::test_support::system_admin_proof_for(&pool, *admin).await;
        let new = svc::rebind(
            &pool,
            &proof,
            &RebindMachineRequest {
                client_id: "window-new".to_string(),
                from_machine_client_id: old,
                label: "steward".to_string(),
                keep_old_active: true,
            },
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("rebind with an overlap window");

        let human = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
        let standing = |pool: PgPool| async move {
            sqlx::query_scalar::<_, String>(
                "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
            )
            .bind(profile)
            .fetch_one(&pool)
            .await
            .expect("standing")
        };

        crate::services::machine_client_service::revoke(&pool, old, &human)
            .await
            .expect("an admin revokes the old credential");
        assert_eq!(
            standing(pool.clone()).await,
            "approved",
            "the surviving credential is still admitted"
        );

        crate::services::machine_client_service::revoke(&pool, new.id, &human)
            .await
            .expect("an admin revokes the last credential");
        assert_eq!(
            standing(pool.clone()).await,
            "revoked",
            "revoking the last live credential revokes standing"
        );
    }

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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &provision_req,
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("provision");

        // Alice owns the machine's team — she can revoke it (tested elsewhere) — but she may NOT
        // rebind it onto a client_id she controls and inherit its identity. Post-enclosure the bar is
        // structural: rebind requires a `&SystemAdmin`, and a non-admin cannot mint one. The refusal
        // now happens at the proof gate, before rebind is even reachable.
        let alice_authed = crate::test_support::human_principal_for(&pool, alice).await;
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("to-be-revoked"),
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("provision");

        crate::services::machine_client_service::revoke(
            &pool,
            old.id,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
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
            crate::auth_config::AuthMode::ExternalIdp,
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &IssueMachineRequest {
                label: "sidekiq".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
            crate::auth_config::AuthMode::ExternalIdp,
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

    /// Seed a non-admin profile that owns a fresh team. Returns (profile, team).
    async fn seed_team_owner(pool: &PgPool, handle: &str) -> (Uuid, Uuid) {
        let team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id",
            format!("{handle}-team"),
        )
        .fetch_one(pool)
        .await
        .expect("team");
        let owner = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
            owner,
            handle,
        )
        .execute(pool)
        .await
        .expect("owner");
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner'::team_role)",
            team,
            owner,
        )
        .execute(pool)
        .await
        .expect("owner owns the team");
        (owner, team)
    }

    async fn machine_count(pool: &PgPool) -> i64 {
        sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM kb_machine_clients"#)
            .fetch_one(pool)
            .await
            .expect("count machines")
    }

    /// [`seed_team_owner`], with the `<handle>@web` emitter a registrar's `machine_reach` event is
    /// authored by.
    async fn seed_registrar(pool: &PgPool, handle: &str) -> (Uuid, Uuid) {
        let (owner, team) = seed_team_owner(pool, handle).await;
        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(&mut conn, owner, handle)
            .await
            .expect("provision registrar emitters");
        (owner, team)
    }

    /// An `issue` request (temper's own AS) for a machine `team` owns, reaching it as `member`.
    fn issue_onto(team: Uuid) -> IssueMachineRequest {
        IssueMachineRequest {
            label: "worker".to_string(),
            owner_team_id: Some(team),
            teams: vec![TeamSpec {
                team_id: team,
                role: "member".to_string(),
            }],
            grants: vec![],
        }
    }

    async fn machine_reach_events(pool: &PgPool) -> Vec<serde_json::Value> {
        sqlx::query_scalar(
            "SELECT e.payload FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'team_membership_changed' AND e.payload->>'via' = 'machine_reach' \
              ORDER BY e.id",
        )
        .fetch_all(pool)
        .await
        .expect("machine reach events")
    }

    /// Task `01a14592-3f83`, BITE: the registrar's bar is decided under the team lock, not on the
    /// pool. R is the team's owner when the registration passes `authorize_registration`; an admin
    /// demotes R to `member` and holds the team lock with that write uncommitted; R's registration
    /// is shown to BLOCK on the team-row lock; the admin commits. Re-resolved under the lock, R no
    /// longer manages the team, so the registration is the pool check's 403: no machine, no
    /// `machine_reach` event naming R. Before the fix, the pool's verdict stood and the machine
    /// landed on the team with R recorded as actor.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_registrar_demoted_while_waiting_on_the_team_lock_is_refused(pool: PgPool) {
        use crate::authz::team_membership::{self, MembershipAct, MembershipAuthority};
        use temper_core::types::team::TeamRole;

        let (registrar, team) = seed_registrar(&pool, "registrar").await;
        let admin = seed_admin(&pool).await;
        // A second owner, so the demotion is not refused as the last owner's (C6).
        let (co_owner, _) = seed_team_owner(&pool, "co-owner").await;
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')",
            team,
            co_owner,
        )
        .execute(&pool)
        .await
        .expect("co-owner");
        let admin_principal = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
        let registrar_principal = crate::test_support::human_principal_for(&pool, registrar).await;

        // Holder: the admin demotes the registrar, through the routed path, written and held open.
        let mut holder = pool.begin().await.unwrap();
        let target = team_membership::locked(&mut holder, team, ProfileId::from(registrar))
            .await
            .unwrap();
        let resolved = MembershipAuthority::resolve(&mut holder, &admin_principal, team)
            .await
            .unwrap();
        let change = team_membership::authorize(
            resolved,
            MembershipAct::Change {
                role: TeamRole::Member,
            },
            &target,
        )
        .unwrap();
        team_membership::membership_write(&mut holder, None, change)
            .await
            .unwrap();

        // Contender: the registrar, still the owner as the pool sees it.
        let contender_pool = pool.clone();
        let contender = tokio::spawn(async move {
            svc::issue(
                &contender_pool,
                &registrar_principal,
                &issue_onto(team),
                crate::auth_config::AuthMode::TemperAs,
            )
            .await
        });
        crate::test_support::await_team_lock_waiter(&pool).await;
        holder.commit().await.unwrap();

        let err = contender
            .await
            .unwrap()
            .expect_err("a registrar demoted under the lock cannot land the reach");
        assert!(
            matches!(err, crate::error::ApiError::Forbidden),
            "the pool check's refusal, got {err:?}"
        );
        assert_eq!(machine_count(&pool).await, 0, "nothing was registered");
        assert!(
            machine_reach_events(&pool).await.is_empty(),
            "no machine_reach event names the demoted registrar"
        );
    }

    /// Task `01a14592-3f83`, PIN: the re-check under the lock admits who the pool check admitted.
    /// A system admin — on a team where they hold no row, so only the admin arm admits them — and
    /// a team's owner each land their reach, each recorded as the actor.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_admin_and_an_owner_still_land_their_reach_under_the_lock(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let (registrar, owned) = seed_registrar(&pool, "registrar").await;
        let (_, elsewhere) = seed_team_owner(&pool, "elsewhere").await;
        let mode = crate::auth_config::AuthMode::TemperAs;

        let by_admin = svc::issue(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &issue_onto(elsewhere),
            mode,
        )
        .await
        .expect("an admin's registration");
        let by_owner = svc::issue(
            &pool,
            &crate::test_support::human_principal_for(&pool, registrar).await,
            &issue_onto(owned),
            mode,
        )
        .await
        .expect("an owner's registration");

        let events = machine_reach_events(&pool).await;
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0]["actor"], serde_json::json!(admin.uuid()));
        assert_eq!(
            events[0]["profile_id"],
            serde_json::json!(by_admin.client.profile_id)
        );
        assert_eq!(events[1]["actor"], serde_json::json!(registrar));
        assert_eq!(
            events[1]["profile_id"],
            serde_json::json!(by_owner.client.profile_id)
        );
    }

    /// Under an external IdP only a system admin registers a client id. A team owner who could
    /// register one they did not create would receive its tokens, so the door refuses them before
    /// it writes anything, even for a machine their own team would own (ruling 3).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn under_an_external_idp_only_an_admin_provisions(pool: PgPool) {
        let (owner, team) = seed_team_owner(&pool, "provisioner").await;
        let mut request = req("someone-elses-client");
        request.owner_team_id = Some(team);

        let err = svc::provision(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            &request,
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect_err("a team owner must not provision under an external IdP");
        assert!(
            matches!(&err, crate::error::ApiError::ForbiddenDetail(m) if m.contains("system admin")),
            "got {err:?}"
        );
        assert_eq!(machine_count(&pool).await, 0, "the refusal wrote nothing");
    }

    /// An instance that runs temper's own AS can authenticate no externally issued client id, so
    /// the doors that write one refuse even an admin, and say which door to use instead. `issue`
    /// stays open there and mints a live credential.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn under_temper_as_the_external_doors_refuse_and_issue_is_live(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let human = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
        let as_mode = crate::auth_config::AuthMode::TemperAs;

        let err = svc::provision(&pool, &human, &req("idp-client"), as_mode)
            .await
            .expect_err("provision is refused under temper's own AS");
        assert!(
            matches!(&err, crate::error::ApiError::BadRequest(m) if m.contains("temper admin machine issue")),
            "got {err:?}"
        );

        let cred = svc::issue(
            &pool,
            &human,
            &IssueMachineRequest {
                label: "worker".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
            as_mode,
        )
        .await
        .expect("issue under temper's own AS");
        assert!(
            !cred.dormant,
            "an AS-mode instance authenticates what it issues"
        );

        let proof = crate::test_support::system_admin_proof_for(&pool, *admin).await;
        let err = svc::rebind(
            &pool,
            &proof,
            &RebindMachineRequest {
                client_id: "idp-client-2".to_string(),
                from_machine_client_id: cred.client.id,
                label: "rebind".to_string(),
                keep_old_active: false,
            },
            as_mode,
        )
        .await
        .expect_err("rebind is refused under temper's own AS");
        assert!(
            matches!(&err, crate::error::ApiError::BadRequest(m) if m.contains("rotate-secret")),
            "got {err:?}"
        );
        assert_eq!(
            machine_count(&pool).await,
            1,
            "only the issued credential exists"
        );
    }

    /// Under an external IdP `issue` is allowed but dormant, so credentials can be staged before an
    /// instance moves to its own AS. A temper-issued machine is never rebound onto an external
    /// client id, in any mode: its secret is rotated instead.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn under_an_external_idp_issue_is_dormant_and_a_temper_machine_is_not_rebound(
        pool: PgPool,
    ) {
        let admin = seed_admin(&pool).await;
        let human = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
        let external = crate::auth_config::AuthMode::ExternalIdp;

        let cred = svc::issue(
            &pool,
            &human,
            &IssueMachineRequest {
                label: "staged".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
            external,
        )
        .await
        .expect("issue is allowed under an external IdP");
        assert!(
            cred.dormant,
            "the response says the credential cannot authenticate yet"
        );

        let proof = crate::test_support::system_admin_proof_for(&pool, *admin).await;
        let err = svc::rebind(
            &pool,
            &proof,
            &RebindMachineRequest {
                client_id: "external-for-a-temper-machine".to_string(),
                from_machine_client_id: cred.client.id,
                label: "rebind".to_string(),
                keep_old_active: false,
            },
            external,
        )
        .await
        .expect_err("a temper-issued source is not rebound");
        assert!(
            matches!(&err, crate::error::ApiError::BadRequest(m) if m.contains("rotate-secret")),
            "got {err:?}"
        );
        let still = crate::services::machine_client_service::get(&pool, cred.client.id)
            .await
            .expect("source row");
        assert!(
            still.revoked_at.is_none(),
            "the refused rebind revoked nothing"
        );
        assert_eq!(machine_count(&pool).await, 1, "and wrote nothing");
    }

    /// A machine is looked up by `client_id` alone, so registering one under an id temper's AS mints
    /// would hand it every token issued to that id: a DCR client's `conn_` tokens, or a future
    /// `tmpr_` credential's. Both caller-supplied doors refuse those namespaces, for an admin too,
    /// and write nothing.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_client_id_temper_mints_is_never_registered_by_hand(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let human = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
        let proof = crate::test_support::system_admin_proof_for(&pool, *admin).await;
        let source = svc::provision(
            &pool,
            &human,
            &req("rebind-source"),
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("an ordinary client id provisions");

        for client_id in ["conn_a1b2c3d4e5f6g7h8", "tmpr_handpicked"] {
            let err = svc::provision(
                &pool,
                &human,
                &req(client_id),
                crate::auth_config::AuthMode::ExternalIdp,
            )
            .await
            .expect_err("provision must refuse a minted namespace");
            assert!(
                matches!(&err, crate::error::ApiError::BadRequest(m) if m.contains(client_id)),
                "provision of {client_id}: expected a 400 naming the id, got {err:?}"
            );

            let err = svc::rebind(
                &pool,
                &proof,
                &RebindMachineRequest {
                    client_id: client_id.to_string(),
                    from_machine_client_id: source.id,
                    label: "rebound".to_string(),
                    keep_old_active: false,
                },
                crate::auth_config::AuthMode::ExternalIdp,
            )
            .await
            .expect_err("rebind must refuse a minted namespace");
            assert!(
                matches!(&err, crate::error::ApiError::BadRequest(m) if m.contains(client_id)),
                "rebind to {client_id}: expected a 400 naming the id, got {err:?}"
            );

            let written: i64 = sqlx::query_scalar(
                "SELECT (SELECT count(*) FROM kb_machine_clients WHERE client_id = $1) \
                      + (SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider_user_id = $1)",
            )
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .expect("count rows");
            assert_eq!(written, 0, "a refused {client_id} wrote nothing");
        }

        let source_row = crate::services::machine_client_service::get(&pool, source.id)
            .await
            .expect("source row");
        assert!(
            source_row.revoked_at.is_none(),
            "a refused rebind left its source live"
        );
    }

    /// The schema keeps the namespaces apart whatever writes the row
    /// (`20261030100000_machine_client_id_namespaces`): a DCR client id begins `conn_`, and a machine
    /// row temper did not issue keeps out of `conn_` and `tmpr_`.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_database_keeps_machine_and_dcr_client_ids_apart(pool: PgPool) {
        fn violates(err: &sqlx::Error, constraint: &str) -> bool {
            err.as_database_error()
                .and_then(|d| d.constraint())
                .is_some_and(|c| c == constraint)
        }

        let err = sqlx::query(
            "INSERT INTO kb_oauth_dcr_clients \
               (client_id, client_secret_hash, grant_types, redirect_uris, token_endpoint_auth_method) \
             VALUES ('tmpr_not_a_dcr_id', 'h', ARRAY['client_credentials'], ARRAY[]::text[], 'client_secret_basic')",
        )
        .execute(&pool)
        .await
        .expect_err("a DCR client id outside conn_ is refused");
        assert!(
            violates(&err, "kb_oauth_dcr_clients_client_id_namespace"),
            "{err}"
        );

        let admin = seed_admin(&pool).await;
        let source = svc::provision(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("schema-source"),
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("an ordinary client id provisions");
        for (client_id, issuer, refused) in [
            ("conn_squatted", "auth0-m2m", true),
            ("tmpr_squatted", "auth0-m2m", true),
            ("tmpr_issued_here", "temper", false),
        ] {
            let result = sqlx::query(
                "INSERT INTO kb_machine_clients \
                   (client_id, issuer, label, profile_id, registered_by_profile_id) \
                 VALUES ($1, $2, 'probe', $3, $4)",
            )
            .bind(client_id)
            .bind(issuer)
            .bind(source.profile_id)
            .bind(*admin)
            .execute(&pool)
            .await;
            match (refused, result) {
                (true, Err(err)) => assert!(
                    violates(&err, "kb_machine_clients_client_id_namespace"),
                    "{client_id} ({issuer}): {err}"
                ),
                (true, Ok(_)) => panic!("{client_id} ({issuer}) must be refused"),
                (false, Err(err)) => panic!("{client_id} ({issuer}) must be admitted: {err}"),
                (false, Ok(_)) => {}
            }
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provisioning_a_duplicate_client_id_is_a_conflict(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        svc::provision(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("dupe"),
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("first");
        let err = svc::provision(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("dupe"),
            crate::auth_config::AuthMode::ExternalIdp,
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &req("attributed-agent"),
            crate::auth_config::AuthMode::ExternalIdp,
        )
        .await
        .expect("provision");
        let issued = svc::issue(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            &IssueMachineRequest {
                label: "attributed-issue".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
            crate::auth_config::AuthMode::ExternalIdp,
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
                &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
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
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
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

    // ── A machine never governs (20261019100000_machines_never_govern.sql) ──────────────────

    /// The SQLSTATE a refusing trigger raises. Asserting it (not just "an error") is what makes a
    /// DB witness bite on its own trigger rather than on any failure.
    fn is_check_violation(err: &sqlx::Error) -> bool {
        err.as_database_error()
            .and_then(|d| d.code())
            .is_some_and(|c| c == "23514")
    }

    async fn personal_team_role(pool: &PgPool, profile: Uuid) -> String {
        sqlx::query_scalar!(
            r#"SELECT tm.role::text AS "role!" FROM kb_team_members tm
                 JOIN kb_teams t ON t.id = tm.team_id
                WHERE t.personal_of = $1 AND tm.profile_id = $1"#,
            profile,
        )
        .fetch_one(pool)
        .await
        .expect("personal team row")
    }

    async fn plain_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id",
            slug,
        )
        .fetch_one(pool)
        .await
        .expect("team")
    }

    /// Registration writes the personal-team `owner` row before the client row exists; the client
    /// row's insert must cap it.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_new_machine_holds_its_personal_team_as_member(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "capped-agent").await;
        assert_eq!(personal_team_role(&pool, machine).await, "member");
    }

    /// The cap is about becoming a machine, not about registration: a profile already holding
    /// `maintainer` elsewhere is capped the moment a client row names it.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn becoming_a_machine_caps_every_role_the_profile_holds(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let profile: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('soon-agent', 'Soon') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("profile");
        let team = plain_team(&pool, "elsewhere").await;
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'maintainer')",
            team,
            profile,
        )
        .execute(&pool)
        .await
        .expect("maintainer while still a person");

        sqlx::query!(
            "INSERT INTO kb_machine_clients \
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id) \
             VALUES ('soon-agent', 'auth0-m2m', 'soon', $1, NULL, $2)",
            profile,
            *admin,
        )
        .execute(&pool)
        .await
        .expect("become a machine");

        let above: i64 = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM kb_team_members
                WHERE profile_id = $1 AND role < 'member'::team_role"#,
            profile,
        )
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(above, 0, "no role above member survives becoming a machine");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_database_refuses_a_machine_above_member(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "floor-agent").await;
        let team = plain_team(&pool, "floor").await;

        let err = sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'maintainer')",
            team,
            machine,
        )
        .execute(&pool)
        .await
        .expect_err("a machine cannot be inserted above member");
        assert!(is_check_violation(&err), "{err}");

        let err = sqlx::query!(
            r#"UPDATE kb_team_members SET role = 'owner'
                WHERE profile_id = $1
                  AND team_id = (SELECT id FROM kb_teams WHERE personal_of = $1)"#,
            machine,
        )
        .execute(&pool)
        .await
        .expect_err("a machine cannot be raised above member, even on its personal team");
        assert!(is_check_violation(&err), "{err}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_database_refuses_a_machine_a_governance_grant(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "gov-agent").await;
        let err = sqlx::query_scalar!(
            "SELECT principal_governance_set($1, true, $2, NULL)",
            machine,
            *admin,
        )
        .fetch_one(&pool)
        .await
        .expect_err("a machine cannot hold governance");
        assert!(is_check_violation(&err), "{err}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_governing_profile_cannot_become_a_machine(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let err = sqlx::query!(
            "INSERT INTO kb_machine_clients \
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id) \
             VALUES ('admin-agent', 'auth0-m2m', 'admin', $1, NULL, $1)",
            *admin,
        )
        .execute(&pool)
        .await
        .expect_err("a profile holding governance cannot become a machine");
        assert!(is_check_violation(&err), "{err}");
    }

    // ── The floor holds against accidental SQL (20261024100000) ──────────────────────────────

    async fn still_a_machine(pool: &PgPool, profile: Uuid) -> bool {
        sqlx::query_scalar!(r#"SELECT is_machine_profile($1) AS "m!""#, profile)
            .fetch_one(pool)
            .await
            .expect("is_machine_profile")
    }

    /// Deleting the client row would make the profile a person again everywhere. Revocation is
    /// the way a machine's credential ends, and it must keep working.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_machine_client_row_is_never_deleted(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "kept-agent").await;

        let err = sqlx::query!(
            "DELETE FROM kb_machine_clients WHERE profile_id = $1",
            machine
        )
        .execute(&pool)
        .await
        .expect_err("a client row cannot be deleted");
        assert!(is_check_violation(&err), "{err}");
        assert!(still_a_machine(&pool, machine).await);

        sqlx::query!(
            "UPDATE kb_machine_clients SET revoked_at = now() WHERE profile_id = $1",
            machine,
        )
        .execute(&pool)
        .await
        .expect("revocation stays the way a credential ends");
        assert!(
            still_a_machine(&pool, machine).await,
            "a revoked machine is still a machine"
        );
    }

    /// Moving the client row to another profile would make its old profile a person again.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_machine_client_row_never_moves_to_another_profile(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "moved-agent").await;
        let other: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('move-target', 'Target') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("profile");

        let err = sqlx::query!(
            "UPDATE kb_machine_clients SET profile_id = $2 WHERE profile_id = $1",
            machine,
            other,
        )
        .execute(&pool)
        .await
        .expect_err("a client row cannot move");
        assert!(is_check_violation(&err), "{err}");
        assert!(still_a_machine(&pool, machine).await);
    }

    /// A role added to `team_role` lands last in the enum, below `watcher`. Enum order would rank
    /// it "not above member" and admit it for a machine; the allowlist refuses it until someone
    /// decides where it ranks. Both the ceiling and the cap must refuse, not admit or rewrite.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_role_added_to_the_enum_is_refused_for_a_machine(pool: PgPool) {
        sqlx::query("ALTER TYPE team_role ADD VALUE 'probe_role'")
            .execute(&pool)
            .await
            .expect("add an enum value");
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "enum-agent").await;
        let team = plain_team(&pool, "enum-team").await;

        let err = sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'probe_role')",
        )
        .bind(team)
        .bind(machine)
        .execute(&pool)
        .await
        .expect_err("the ceiling refuses a role it does not know");
        assert!(is_check_violation(&err), "{err}");

        // The cap side: a person holding the unknown role cannot become a machine.
        let person: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('enum-person', 'Person') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("profile");
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'probe_role')",
        )
        .bind(team)
        .bind(person)
        .execute(&pool)
        .await
        .expect("a person may hold it");
        let err = sqlx::query!(
            "INSERT INTO kb_machine_clients \
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id) \
             VALUES ('enum-person', 'auth0-m2m', 'enum', $1, NULL, $2)",
            person,
            *admin,
        )
        .execute(&pool)
        .await
        .expect_err("the cap refuses a role it does not know how to lower");
        assert!(is_check_violation(&err), "{err}");
    }

    /// Waits until a transaction in this test's database is blocked on an advisory lock, which is
    /// the per-profile lock the machine triggers take. Polling `pg_locks` rather than sleeping
    /// makes the proof independent of load: without the lock the second writer never waits, so it
    /// finishes, and this fails, however slowly the machine runs.
    async fn wait_until_blocked_on_the_profile_lock<T>(
        pool: &PgPool,
        writer: &tokio::task::JoinHandle<T>,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            assert!(
                !writer.is_finished(),
                "the second writer finished without waiting on the profile's lock"
            );
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_locks \
                                 WHERE locktype = 'advisory' AND NOT granted \
                                   AND database = (SELECT oid FROM pg_database \
                                                    WHERE datname = current_database()))",
            )
            .fetch_one(pool)
            .await
            .expect("pg_locks");
            if waiting {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the second writer never reached the profile's lock"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// Two transactions writing a governance grant and a client row for the same person must not
    /// both commit. Without the per-profile lock, neither trigger sees the other's uncommitted
    /// row and both pass. With it, the second writer waits for the first, then reads its commit.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_concurrent_governance_grant_and_client_row_cannot_both_commit(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let person: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('race-person', 'Race') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("profile");

        let mut governs = pool.begin().await.expect("tx 1");
        sqlx::query_scalar!(
            "SELECT principal_governance_set($1, true, $2, NULL)",
            person,
            *admin,
        )
        .fetch_one(&mut *governs)
        .await
        .expect("tx 1 grants governance while the profile is a person");

        let becomes = {
            let pool = pool.clone();
            let admin = *admin;
            tokio::spawn(async move {
                let mut tx = pool.begin().await.expect("tx 2");
                sqlx::query!(
                    "INSERT INTO kb_machine_clients \
                       (client_id, issuer, label, profile_id, team_id, registered_by_profile_id) \
                     VALUES ('race-person', 'auth0-m2m', 'race', $1, NULL, $2)",
                    person,
                    admin,
                )
                .execute(&mut *tx)
                .await?;
                tx.commit().await
            })
        };

        wait_until_blocked_on_the_profile_lock(&pool, &becomes).await;
        governs.commit().await.expect("tx 1 commits");

        let err = becomes
            .await
            .expect("join tx 2")
            .expect_err("tx 2 reads tx 1's governance grant once the lock is released");
        assert!(is_check_violation(&err), "{err}");
        assert!(!still_a_machine(&pool, person).await);
    }

    /// The ceiling's half of the race: a client row and an `owner` row for the same person, in
    /// two transactions. Without the ceiling's lock, the cap runs before the `owner` row exists
    /// and the ceiling cannot see the uncommitted client row, so both commit and a machine owns
    /// a team. With it, the `owner` insert waits, then reads the committed client row.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_concurrent_client_row_and_owner_row_cannot_both_commit(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let person: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('race-owner', 'Race') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("profile");
        let team = plain_team(&pool, "race-team").await;

        let mut becomes = pool.begin().await.expect("tx 1");
        sqlx::query!(
            "INSERT INTO kb_machine_clients \
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id) \
             VALUES ('race-owner', 'auth0-m2m', 'race', $1, NULL, $2)",
            person,
            *admin,
        )
        .execute(&mut *becomes)
        .await
        .expect("tx 1 makes the person a machine");

        let owns = {
            let pool = pool.clone();
            tokio::spawn(async move {
                let mut tx = pool.begin().await.expect("tx 2");
                sqlx::query!(
                    "INSERT INTO kb_team_members (team_id, profile_id, role) \
                     VALUES ($1, $2, 'owner')",
                    team,
                    person,
                )
                .execute(&mut *tx)
                .await?;
                tx.commit().await
            })
        };

        wait_until_blocked_on_the_profile_lock(&pool, &owns).await;
        becomes.commit().await.expect("tx 1 commits");

        let err = owns
            .await
            .expect("join tx 2")
            .expect_err("tx 2 reads tx 1's client row once the lock is released");
        assert!(is_check_violation(&err), "{err}");
    }

    /// A table created earlier in a caller's `search_path` must not stand in for the real ones.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_machine_floor_functions_pin_their_search_path(pool: PgPool) {
        let names = [
            "is_machine_profile",
            "_machine_principal_lock",
            "machine_team_role_ceiling",
            "machine_never_governs",
            "machine_client_caps_profile",
            "machine_client_row_is_permanent",
        ]
        .map(String::from);
        let rows = sqlx::query!(
            r#"SELECT p.proname::text AS "name!",
                      EXISTS (SELECT 1 FROM unnest(coalesce(p.proconfig, '{}')) c
                               WHERE c LIKE 'search_path=%') AS "pinned!"
                 FROM pg_proc p
                WHERE p.proname = ANY($1)
                ORDER BY 1"#,
            &names[..],
        )
        .fetch_all(&pool)
        .await
        .expect("pg_proc");
        // Every name must be found, or an absent function would read as "nothing unpinned".
        assert_eq!(rows.len(), names.len(), "found: {rows:?}");
        let unpinned: Vec<_> = rows.iter().filter(|r| !r.pinned).map(|r| &r.name).collect();
        assert!(unpinned.is_empty(), "unpinned: {unpinned:?}");
    }

    /// Each service refusal is asserted by its own message, so with the service guard removed the
    /// trigger's error (a different variant) fails the test: each witness bites on its own guard.
    fn is_machine_refusal(err: &crate::error::ApiError) -> bool {
        matches!(err, crate::error::ApiError::BadRequest(m) if m.contains("machine principal"))
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn add_member_refuses_a_machine_above_member(pool: PgPool) {
        use temper_core::types::team::{AddMemberRequest, TeamRole};
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "add-agent").await;
        let team = plain_team(&pool, "add-team").await;
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')",
            team,
            *admin,
        )
        .execute(&pool)
        .await
        .expect("admin owns the team");

        let err = crate::services::team_service::add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            &AddMemberRequest {
                profile_id: machine,
                role: TeamRole::Maintainer,
            },
        )
        .await
        .expect_err("a machine cannot be added as maintainer");
        assert!(is_machine_refusal(&err), "{err:?}");

        crate::services::team_service::add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            &AddMemberRequest {
                profile_id: machine,
                role: TeamRole::Member,
            },
        )
        .await
        .expect("a machine may be added as member");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn change_role_refuses_a_machine_above_member(pool: PgPool) {
        use temper_core::types::team::TeamRole;
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "role-agent").await;
        let team = plain_team(&pool, "role-team").await;
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'owner'), ($1, $3, 'member')",
            team,
            *admin,
            machine,
        )
        .execute(&pool)
        .await
        .expect("admin owns the team; the machine is a member");

        let err = crate::services::team_service::change_role(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            machine,
            TeamRole::Maintainer,
        )
        .await
        .expect_err("a machine cannot be raised to maintainer");
        assert!(is_machine_refusal(&err), "{err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn promote_admin_refuses_a_machine(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "promo-agent").await;
        let gate = crate::auth::require_system_admin_by_id(&pool, admin)
            .await
            .expect("the seeded admin is an admin");
        let err = access_service::promote_admin(&pool, &gate, machine, None)
            .await
            .expect_err("a machine cannot be promoted");
        assert!(is_machine_refusal(&err), "{err:?}");
    }

    /// As `member` the last-owner guard no longer holds a machine on its personal team; this
    /// refusal does. The machine cannot leave by itself (`remove_member` takes a person), so the
    /// witness is a person who manages the team removing it.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_machine_is_never_removed_from_its_personal_team(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "stay-agent").await;
        let team: Uuid =
            sqlx::query_scalar!("SELECT id FROM kb_teams WHERE personal_of = $1", machine,)
                .fetch_one(&pool)
                .await
                .expect("personal team");
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')",
            team,
            *admin,
        )
        .execute(&pool)
        .await
        .expect("a person manages the machine's personal team");
        let err = crate::services::team_service::remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            machine,
        )
        .await
        .expect_err("a machine is never removed from its personal team");
        assert!(
            matches!(&err, crate::error::ApiError::Conflict(m) if m.contains("personal team")),
            "{err:?}"
        );
    }

    /// Pin, not a bite: capping the personal team at `member` takes nothing a machine uses on its
    /// own content — its default context is profile-owned.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_capped_machine_still_authors_its_default_context(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let machine = approved_machine(&pool, admin, "author-agent").await;
        let authors: bool = sqlx::query_scalar!(
            r#"SELECT context_authorable_by_profile($1, c.id) AS "a!: bool"
                 FROM kb_contexts c
                WHERE c.owner_table = 'kb_profiles' AND c.owner_id = $1 AND c.slug = 'default'"#,
            machine,
        )
        .fetch_one(&pool)
        .await
        .expect("default context");
        assert!(authors);
    }
}
