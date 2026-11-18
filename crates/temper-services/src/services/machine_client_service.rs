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

use temper_principal::Standing;

use crate::auth::HumanPrincipal;
use crate::authz::{MachineClientControlAuthority, Principal};
use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, standing_service};

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
                  revoked_at, revoked_by_profile_id, secret_rotated_by_profile_id
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
/// `client_id`: by the time a gate runs, the `client_id` is gone —
/// `profile_service::resolve_machine_from_claims` exchanged it for `client.profile_id` and every
/// downstream layer carries only that. Set 5's audit gate needs the same fact at a *scoped*
/// decision, so it asks here rather than re-deriving it from claims it does not have.
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
/// that back them (20261019100000_machines_never_govern.sql) share one definition.
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
                  revoked_at, revoked_by_profile_id, secret_rotated_by_profile_id
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
    authed: &HumanPrincipal,
    id: Uuid,
) -> ApiResult<MachineClient> {
    authorize_control(pool, authed, id).await?;
    get(pool, id).await
}

/// The per-row gate every act on an existing machine client passes first: a system admin, or the
/// owner of the machine's owning team, keyed on the row (B2 D5). A refusal is indistinguishable
/// from a missing id. Ahead of any write, so a rejected act never touches the row.
///
/// Answers whether the decision resolved on the system-admin arm, which a writing act asks again
/// under the governance lock on its transaction (`auth::admin_arm_holds`).
async fn authorize_control(pool: &PgPool, authed: &HumanPrincipal, id: Uuid) -> ApiResult<bool> {
    Ok(
        crate::authz::authorize::<MachineClientControlAuthority>(
            pool,
            Principal::Human(authed),
            id,
        )
        .await?
        .authority()
            == MachineClientControlAuthority::SystemAdmin,
    )
}

/// List machine clients visible to `caller` (B2 D5), newest first. Revoked rows are hidden
/// unless asked for. A system admin sees every row, including teamless ones and those of a personal
/// team (they manage every principal, task `01a191b4`); a team owner sees only machines owned by a
/// team they own.
///
/// `EXISTS`, not `array_agg` — an empty scope must DENY, and an aggregate over an empty scope
/// yields NULL, which falls open.
pub async fn list(
    pool: &PgPool,
    authed: &HumanPrincipal,
    include_revoked: bool,
) -> ApiResult<Vec<MachineClient>> {
    let caller = ProfileId::from(authed.profile().id);
    let is_admin = crate::authz::Principal::Human(authed)
        .system_admin(pool)
        .await?
        .is_some();

    let rows = sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id, secret_rotated_by_profile_id
             FROM kb_machine_clients mc
            WHERE ($1 OR mc.revoked_at IS NULL)
              AND ( $2
                    OR ( mc.team_id IS NOT NULL
                         AND EXISTS (
                             SELECT 1
                               FROM kb_team_members tm
                               JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
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

/// Mark a client dead. Grants and memberships are deliberately untouched (D11). A second revoke
/// of an already-revoked row leaves the row as it was (the first revoker and first timestamp are
/// the truth), but an admin's second revoke still runs the standing arm below: an admin can finish
/// what an owner's revoke left.
///
/// **What it does to standing depends on who asks.**
///
/// - A system admin's revoke of a profile's **last** live credential also revokes the machine's
///   standing (D17: one revocation fact, not two that can drift). While another credential on the
///   profile is still live (the overlap window `rebind` opens with `keep_old_active`), standing is
///   left alone: the machine is still admitted through that one, and revoking standing would cut
///   it off too.
/// - A team owner's revoke kills the login credential only (ruled 2026-10-09: "revoking a
///   credential revokes the login credential only"). `Act::Revoke` on standing is an admin act,
///   and an owner holds no [`SystemAdmin`](crate::auth::SystemAdmin) proof to perform it with.
///   Standing stays as an admin set it, and admits nothing once no live credential remains: a
///   revoked row never authenticates.
pub async fn revoke(pool: &PgPool, id: Uuid, authed: &HumanPrincipal) -> ApiResult<MachineClient> {
    let revoker = ProfileId::from(authed.profile().id);
    // Auth before writes, keyed on the existing row's owning team (B2 D5).
    authorize_control(pool, authed, id).await?;
    let existing = get(pool, id).await?;
    // `Some` only for a system admin. Holding the proof is what lets the standing arm below run.
    let admin = Principal::Human(authed).system_admin(pool).await?;
    let subject = ProfileId::from(existing.profile_id);

    // One transaction for the credential and, for an admin, the standing arm, so a refusal of the
    // standing arm never leaves the credential half of the act committed. The authority is decided
    // again on it, before any write:
    // - for an admin, the subject's standing row is locked first (the order every standing act
    //   takes), then the admin's standing is asked again under the governance lock. An admin
    //   demoted since the pool check is refused the whole act, an owner of the team included; a
    //   retry decides on the owner arm.
    // - otherwise, the owner arm is asked again under the owning team's row lock (the lock every
    //   membership writer takes): a caller who passed the pool check as an admin and was demoted
    //   before `admin` was read, or an owner removed since, is refused as a missing row is.
    let mut tx = pool.begin().await?;
    if let Some(admin) = &admin {
        standing_service::lock_conn(&mut tx, subject).await?;
        // A demoted admin is answered as a caller with no authority over the row is: the same
        // sentence as a missing id, so the refusal says nothing about whether this one exists.
        access_service::still_admin(&mut tx, admin, Some(subject))
            .await
            .map_err(|e| match e {
                ApiError::Forbidden => ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string()),
                other => other,
            })?;
    } else {
        let owns = match existing.team_id {
            Some(team_id) => {
                crate::authz::team_membership::lock_team(&mut tx, team_id).await?
                    && crate::services::team_service::role_on_team_on(&mut tx, team_id, revoker)
                        .await?
                        == Some(temper_core::types::team::TeamRole::Owner)
            }
            None => false,
        };
        if !owns {
            return Err(ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string()));
        }
    }
    sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *revoker,
    )
    .execute(&mut *tx)
    .await?;

    // D17 — for an admin, one revocation fact, not two that can drift. `revoked_at` is an
    // AUTHENTICATION detail (a revoked credential is rejected at profile resolution and never
    // reaches admission); standing is the ADMISSION fact, and it belongs to the profile, which a
    // second live credential may still be using.
    //
    // Load-first, not fire-and-swallow: Revoke is legal ONLY from `Approved` (transition.rs), and a
    // machine is born `Denied` (D11) and may never have been approved. Firing unconditionally would
    // fail the credential revocation on that illegal cell, and the credential is what the operator
    // asked to kill. A standing arm that does run and is refused (the admin's re-check under its
    // locks) rolls the credential back with it: the act is one, and its answer is the refusal.
    // Routing through `access_service::admin_revoke`
    // means the act carries the admin's own proof, and `standing_service::apply` brings the demotion
    // hook and the typed refusal with it. Grants and memberships are deliberately left intact (D11):
    // Revoke denies ADMISSION, which sits above them, so a later rebind still cannot silently
    // resurrect them.
    if let Some(admin) = admin {
        if !has_live_credential(&mut tx, subject).await?
            && standing_service::load_conn(&mut tx, subject).await? == Some(Standing::Approved)
        {
            access_service::admin_revoke_on(
                &mut tx,
                &admin,
                subject,
                format!("machine client {} revoked", existing.client_id),
            )
            .await?;
        }
    }
    tx.commit().await?;

    get(pool, id).await
}

/// Whether any credential on this agent profile is still live.
async fn has_live_credential(conn: &mut sqlx::PgConnection, profile: ProfileId) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM kb_machine_clients WHERE profile_id = $1 AND revoked_at IS NULL
           ) AS "live!""#,
        *profile,
    )
    .fetch_one(&mut *conn)
    .await?)
}

/// The proof that a machine's personal-team membership is being severed by a credential mint
/// (task `01a191b4`). Private field: only [`sever_personal_reach`] constructs it, so
/// `AuthorizedMembershipChange::severed_by_credential` runs for no other caller.
#[derive(Debug)]
pub(crate) struct CredentialSeverance(());

/// The personal teams, of persons other than `minter`, that admit this machine profile.
pub(crate) async fn others_personal_teams(
    conn: &mut sqlx::PgConnection,
    profile: Uuid,
    minter: Uuid,
) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar!(
        r#"SELECT t.id
             FROM kb_team_members tm
             JOIN kb_teams t ON t.id = tm.team_id
            WHERE tm.profile_id = $1
              AND profile_is_person(t.personal_of)
              AND t.personal_of <> $2
            ORDER BY t.id"#,
        profile,
        minter,
    )
    .fetch_all(&mut *conn)
    .await?)
}

/// Sever `profile` from every person's personal team but the minter's own, in the minting
/// transaction and before its writes (task `01a191b4`).
///
/// A machine's credential is a key to every team it sits on, and a person's personal team is theirs
/// alone. A system admin manages every principal, machines included, and an owner of the team that
/// owns a machine row may rotate its secret; neither may thereby open someone's personal space. So
/// a credential minted by anyone but the person whose personal team admits the machine (a rotated
/// secret, or a rebound client id) costs the machine that team. Each removal is a recorded
/// `team_membership_changed` (`machine_reach`, the minter as actor). Unless the minter is the
/// machine's own person, its direct grants on anyone else's personal space go too, each a recorded
/// `grant_revoked` (task `01a1933d`; ported from temper#1283, narrowed by the 2026-10-31 ruling).
/// Returns the teams severed.
pub(crate) async fn sever_personal_reach(
    conn: &mut sqlx::PgConnection,
    profile: Uuid,
    minter: ProfileId,
) -> ApiResult<Vec<Uuid>> {
    let teams = others_personal_teams(conn, profile, *minter).await?;
    sever_personal_reach_from(conn, profile, minter, teams).await
}

/// [`sever_personal_reach`], over a list of teams the caller already read with
/// [`others_personal_teams`] (and may already hold locked), so one list decides both what was
/// locked and what is severed.
async fn sever_personal_reach_from(
    conn: &mut sqlx::PgConnection,
    profile: Uuid,
    minter: ProfileId,
    teams: Vec<Uuid>,
) -> ApiResult<Vec<Uuid>> {
    use crate::authz::team_membership::{self, AuthorizedMembershipChange};
    for team in &teams {
        let target = team_membership::locked(conn, *team, ProfileId::from(profile)).await?;
        if target.current().is_none() {
            continue;
        }
        let change = AuthorizedMembershipChange::severed_by_credential(
            CredentialSeverance(()),
            &target,
            minter,
        )?;
        team_membership::membership_write(conn, None, change).await?;
    }

    // The machine's own grants on someone else's personal space go with it (task `01a1933d`): a
    // grant on a person's private map (a registration's reach) or on their personal context or a
    // resource homed in one would otherwise still open it to whoever holds the new credential.
    // Each is a recorded `grant_revoked`, the minter as revoker. The minter's own space stays.
    //
    // Only when the credential leaves its person's hands (ruled 2026-10-31): the machine's own
    // person — the person who registered it, whose personal team admits it — rotating or
    // rebinding it withdraws nothing, as the team severance above leaves their own team alone.
    let own_person = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM kb_machine_clients
                           WHERE profile_id = $1 AND registered_by_profile_id = $2
                             AND profile_is_person($2)) AS "own!: bool""#,
        profile,
        *minter,
    )
    .fetch_one(&mut *conn)
    .await?;
    if own_person {
        return Ok(teams);
    }
    let grants = sqlx::query!(
        r#"SELECT subject_table AS "subject_table!", subject_id AS "subject_id!"
             FROM kb_access_grants
            WHERE principal_table = 'kb_profiles' AND principal_id = $1
              AND COALESCE(CASE subject_table
                             WHEN 'kb_cogmaps'   THEN cogmap_personal_owner(subject_id)
                             WHEN 'kb_contexts'  THEN context_personal_owner(subject_id)
                             WHEN 'kb_resources' THEN resource_personal_owner(subject_id)
                           END <> $2, false)
            ORDER BY subject_table, subject_id
              FOR UPDATE"#,
        profile,
        *minter,
    )
    .fetch_all(&mut *conn)
    .await?;
    if !grants.is_empty() {
        let emitter = temper_substrate::writes::resolve_emitter(&mut *conn, minter, "web")
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let seal = CredentialSeverance(());
        for grant in grants {
            let Some(subject) = crate::authz::wire_subject(&grant.subject_table, grant.subject_id)
            else {
                continue;
            };
            crate::services::access_service::delete_grant(
                conn,
                &crate::authz::RevokeWarrant::Severance {
                    _seal: &seal,
                    subject,
                },
                "kb_profiles",
                profile,
                minter,
                emitter,
            )
            .await?;
        }
    }
    Ok(teams)
}

/// The longest a rotated-away secret may remain valid. A rotation window is meant to be brief
/// (issue new → deploy → old expires); an unbounded grace would keep a possibly-compromised old
/// secret alive far past the rotation's intent, defeating D6's "two live secrets, briefly".
const MAX_ROTATION_GRACE_SECONDS: i64 = 7 * 24 * 3_600;

/// Rotate a temper-issued secret (Phase B1, D6). Moves the current secret to `previous` with a
/// grace window, installs a fresh current, and returns the new plaintext once. Rejects a client
/// that temper did not issue (its secret lives at its IdP), one already revoked, or a grace
/// window outside `[0, MAX_ROTATION_GRACE_SECONDS]`.
///
/// Under an external IdP the rotated credential is still **dormant** (see
/// `machine_registration_service::issue`), and the response says so.
pub async fn rotate_secret(
    pool: &PgPool,
    authed: &HumanPrincipal,
    id: Uuid,
    grace_seconds: i64,
    mode: crate::auth_config::AuthMode,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    // Auth before writes, keyed on the existing row's owning team (B2 D5). Ahead of the
    // transaction, so a rejected rotation never touches the row.
    let on_admin_arm = authorize_control(pool, authed, id).await?;

    if !(0..=MAX_ROTATION_GRACE_SECONDS).contains(&grace_seconds) {
        return Err(ApiError::BadRequest(format!(
            "grace_seconds must be between 0 and {MAX_ROTATION_GRACE_SECONDS} (7 days); got {grace_seconds}"
        )));
    }

    let existing = get(pool, id).await?;
    let profile = existing.profile_id;
    let rotator = ProfileId::from(authed.profile().id);
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // The admin arm was decided on the pool; a rotation hands back a live secret for the machine,
    // so it is asked again first, under the governance lock and before the severance's writes. A
    // demoted admin is refused as a caller with no authority over the row is (`404`, the missing-id
    // sentence); one who also owns the machine's team is admitted by a retry on the owner arm.
    // Lock order: governance, then the severed personal teams, then the credential row. An admin's
    // `revoke` takes the machine's standing row, then governance, then the credential row, and no
    // team; an owner's takes the owning team, then the credential row. Every act that takes a team
    // row and the credential row takes the team first, and a demotion takes none of these rows.
    if !crate::auth::admin_arm_holds(&mut tx, rotator, on_admin_arm).await? {
        return Err(ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string()));
    }

    // The owner arm, likewise decided on the pool, is asked again under the owning team's row lock,
    // with the personal teams the severance below will lock taken in the same ascending pass. An
    // owner removed from the team (or demoted) since the pool check, or while this runs, is refused
    // as a caller with no authority over the row is, and mints no secret. Lock order on this arm:
    // those teams, then the credential row, as an owner's `revoke` takes them.
    // One read of the personal teams decides both the locks below and the severance, so no team
    // the severance takes can be missing from the ascending pass.
    let severed = others_personal_teams(&mut tx, profile, *rotator).await?;
    if !on_admin_arm {
        let owns = match existing.team_id {
            Some(team_id) => {
                crate::authz::team_membership::role_under_lock(&mut tx, team_id, rotator, &severed)
                    .await?
                    == Some(temper_core::types::team::TeamRole::Owner)
            }
            None => false,
        };
        if !owns {
            return Err(ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string()));
        }
    }

    // A secret minted by anyone but the person whose personal team admits this machine costs the
    // machine that team. First after the re-check, so its team locks come before the row lock
    // below, as every membership writer orders them; a refusal below rolls it back.
    sever_personal_reach_from(&mut tx, profile, rotator, severed).await?;

    // FOR UPDATE locks the row so a concurrent revoke cannot land between these guards and the
    // write — the issuer/revoked checks and the rotation see one consistent, pinned row.
    let row = sqlx::query!(
        "SELECT issuer, client_id, revoked_at FROM kb_machine_clients WHERE id = $1 FOR UPDATE",
        id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string()))?;

    if row.issuer != crate::auth::machine_door::TEMPER_ISSUER {
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
                  secret_rotated_at          = now(),
                  secret_rotated_by_profile_id = $4
            WHERE id = $1"#,
        id,
        grace_seconds as f64,
        secret.hash,
        authed.profile().id,
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
        dormant: crate::auth::machine_door::machine_door_policy(
            mode,
            crate::auth::machine_door::MachineDoor::Issue,
        ) == crate::auth::machine_door::DoorPolicy::Dormant,
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
