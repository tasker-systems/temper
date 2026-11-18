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

/// The refusal of `provision` to anyone but a system admin under an external IdP: at the door, and
/// again on its transaction for an admin demoted since the door's pool read.
fn provision_needs_an_admin() -> ApiError {
    ApiError::ForbiddenDetail(
        "registering an externally issued client id requires a system admin on this instance"
            .to_string(),
    )
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
    let admin_only = match machine_door::machine_door_policy(mode, MachineDoor::Provision) {
        DoorPolicy::Refused(why) => return Err(ApiError::BadRequest(why.to_string())),
        DoorPolicy::AdminOnly => match crate::auth::require_system_admin(pool, authed).await {
            Ok(_) => true,
            // Only a genuine non-admin is answered with the reason; a failed read stays a 5xx.
            Err(ApiError::Forbidden) => return Err(provision_needs_an_admin()),
            Err(other) => return Err(other),
        },
        DoorPolicy::Allowed | DoorPolicy::Dormant => false,
    };

    let caller = ProfileId::from(authed.profile().id);
    // Auth before writes: a rejected registration must leave the DB completely unchanged —
    // no orphaned agent profile, no partial enrollment. Resolving before the transaction is
    // what makes that assertable.
    let reach = match machine_authz::authorize_registration(
        pool,
        authed,
        req.owner_team_id,
        &req.teams,
        &req.grants,
    )
    .await
    {
        Ok(reach) => reach,
        // A demotion that commits between the door's read and this one resolves no authority.
        // Under the admin-only door that caller is told what a non-admin is told at the door; an
        // admin refused for the request itself (the role ceiling) keeps its own refusal.
        Err(ApiError::Forbidden)
            if admin_only
                && !crate::services::access_service::is_system_admin(pool, caller).await? =>
        {
            return Err(provision_needs_an_admin());
        }
        Err(other) => return Err(other),
    };
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

    // The door's admin bar and the reach's admin arm were read on the pool. Ask again on the
    // transaction, under the governance lock and before any write or the conflict probe, so an admin
    // demoted since (or while this runs) registers nothing, and is told what a non-admin is told.
    if admin_only {
        if !crate::auth::is_system_admin_locked(&mut tx, caller).await? {
            return Err(provision_needs_an_admin());
        }
    } else {
        reach.recheck_admin_arm(&mut tx).await?;
    }
    // No owner-arm re-check is needed here, unlike `issue`. This door is admin-only or refused in
    // every mode (`machine_door_policy`), so whatever arm the reach was authorized on, the
    // registration lands only for a caller who is an admin under the governance lock above.

    // The friendly-conflict check. It is NOT the race guard — two concurrent provisions both
    // pass it. The unique constraints are the guard, and `map_duplicate` turns either one into
    // a 409 naming the client id.
    if sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM kb_machine_clients WHERE client_id = $1) AS "taken!""#,
        req.client_id,
    )
    .fetch_one(&mut *tx)
    .await?
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

    // A reach authorized on the admin arm was read on the pool; ask again under the governance
    // lock, before any write (see `AuthorizedReach::recheck_admin_arm`). One on the owner arm is
    // asked again under the owning team's row lock, its grants with it, also before any write: an
    // owner removed since mints no credential (`AuthorizedReach::recheck_owner_arm`).
    reach.recheck_admin_arm(&mut tx).await?;
    reach.recheck_owner_arm(&mut tx).await?;

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

/// The source rows `rebind` refuses, whoever asks.
///
/// A revoked credential is dead; it must be re-created by a fresh `provision`, never resurrected
/// under a new `client_id`. Rebinding one would revive its surviving grants and memberships (revoke
/// leaves them, D11), silently undoing a deliberate revocation. Mirrors `rotate_secret`'s
/// revoked-source guard.
///
/// A temper-issued machine has a secret to rotate. Rebinding it would move its profile onto an
/// external client id, and on an instance running its own AS that machine stops authenticating.
/// After the revoked check, so a revoked source is told to issue anew rather than to rotate.
fn refuse_unrebindable(client_id: &str, issuer: &str, revoked: bool) -> ApiResult<()> {
    if revoked {
        return Err(ApiError::BadRequest(format!(
            "machine client '{client_id}' is revoked; issue a new credential instead"
        )));
    }
    if issuer == machine_door::TEMPER_ISSUER {
        return Err(ApiError::BadRequest(format!(
            "machine client '{client_id}' was issued by temper; rotate its secret with \
             `temper admin machine rotate-secret` instead"
        )));
    }
    Ok(())
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
///
/// **The admin question is asked again first, on the transaction, under the governance lock.** The
/// `&SystemAdmin` proof was minted on the pool, and a rebind binds a client id the caller chose to a
/// machine's whole reach, so an admin demoted since the mint, or while this runs, must not finish
/// it (`auth::is_system_admin_locked`: a demotion committed first refuses, one in flight
/// makes this wait and then refuses). It comes before every other refusal, so a demoted admin is
/// answered as a non-admin is at the mint (`403`), never with a `404` or `400` about the source row.
///
/// Lock order: the caller's governance row (`FOR KEY SHARE`), then the personal teams the machine
/// is severed from (`sever_personal_reach`: team rows ascending, then member and grant rows), then
/// the source credential row (`FOR UPDATE`). A demotion locks the subject's standing and governance
/// rows and none of these. A machine revoke takes the credential row after its standing and
/// governance locks (admin) or after the owning team's row (owner), and `rotate_secret` after the
/// same severance, so every door that takes a team row and a credential row takes the team first.
pub async fn rebind(
    pool: &PgPool,
    admin: &SystemAdmin,
    req: &RebindMachineRequest,
    mode: AuthMode,
) -> ApiResult<MachineClient> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;
    if !crate::auth::is_system_admin_locked(&mut tx, admin.actor()).await? {
        return Err(ApiError::Forbidden);
    }

    if let DoorPolicy::Refused(why) = machine_door::machine_door_policy(mode, MachineDoor::Rebind) {
        return Err(ApiError::BadRequest(why.to_string()));
    }

    // The new client id is the caller's choice, so it may not sit in a namespace temper mints.
    refuse_minted_namespace(&req.client_id)?;

    // The source, read without a lock: a credential row never changes profile (it is permanent),
    // so the profile read here is the one the writes use. Its revoked and issuer state are checked
    // now, so a request doomed by them is answered before the severance takes anyone's team locks,
    // and again under the row's lock below, which is the check that holds.
    let source = sqlx::query!(
        r#"SELECT profile_id, client_id, issuer, revoked_at
             FROM kb_machine_clients WHERE id = $1"#,
        req.from_machine_client_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        ApiError::NotFound(machine_client_service::MACHINE_CLIENT_REFUSAL.to_string())
    })?;
    refuse_unrebindable(
        &source.client_id,
        &source.issuer,
        source.revoked_at.is_some(),
    )?;
    let profile = source.profile_id;

    // A rebound client id is a new credential for the same principal. One minted by anyone but the
    // person whose personal team admits the machine costs the machine that team: the admin manages
    // the principal, but its new credential does not open someone's personal space. Ahead of the
    // source row's lock, so team rows come before the credential row, as in
    // `rotate_secret`; a refusal below rolls it back.
    machine_client_service::sever_personal_reach(&mut tx, profile, admin.actor()).await?;

    // The source row, locked, so a revoke cannot land between these guards and the writes below.
    let old = sqlx::query!(
        r#"SELECT client_id, issuer, team_id, revoked_at
             FROM kb_machine_clients WHERE id = $1 FOR UPDATE"#,
        req.from_machine_client_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        ApiError::NotFound(machine_client_service::MACHINE_CLIENT_REFUSAL.to_string())
    })?;
    refuse_unrebindable(&old.client_id, &old.issuer, old.revoked_at.is_some())?;

    // A second auth link for the same profile, under the new client id.
    sqlx::query!(
        r#"INSERT INTO kb_profile_auth_links
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
           VALUES ($1, $2, $3, $4, NULL, false, false, now())"#,
        Uuid::now_v7(),
        profile,
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
        profile,
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
            req.from_machine_client_id,
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
mod tests;
