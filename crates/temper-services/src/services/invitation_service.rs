//! Team invitation service over `kb_team_invitations`.
//!
//! Service-direct: no Backend-trait command, no event emission — invitations are
//! provisioning/infra, the same precedent as `team_service` / `context_service`.
//! Authorization follows the rank rule (`authz::team_membership`, C16/R10): creating an
//! invitation requires a managing authority at a role capped at the inviter's own; seeing or
//! revoking one requires a managing authority, an owner-level invitation included; the accept
//! itself is the claimed invitation's act, written under the team lock. Tokens are 128-bit
//! CSPRNG values, never UUIDs (which are time-sortable and guessable).

// rand 0.10 renamed `OsRng` to `SysRng` and made it fallible (`TryRng`); `UnwrapErr` restores
// the infallible `Rng` interface with 0.8's semantics — an OS RNG failure panics.
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use rand::Rng as _;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::HumanPrincipal;
use crate::authz::team_membership::{self, MembershipAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::team_service::{role_on_team, role_on_team_on};
use temper_core::types::ids::ProfileId;
use temper_core::types::invitation::{
    AcceptInvitationResponse, InvitationStatus, InviteeInvitation, PendingInvitationCounts,
    TeamInvitation,
};
use temper_core::types::team::TeamRole;

/// What the accept path's claim `UPDATE` returned: the invitation was live, pending and the
/// caller's, and it is now `accepted`. Private fields: only that claim builds one, so the
/// member write it authorizes cannot run for an invitation nobody claimed (plan P12/P16). It
/// carries the invitee the claim was bound to (`vw_invitee_invitations`), so the write cannot be
/// pointed at anyone else.
#[derive(Debug)]
pub(crate) struct ClaimedInvitation {
    team_id: Uuid,
    role: TeamRole,
    invited_by: ProfileId,
    invitee: ProfileId,
}

impl ClaimedInvitation {
    pub(crate) fn team_id(&self) -> Uuid {
        self.team_id
    }

    pub(crate) fn role(&self) -> TeamRole {
        self.role
    }

    pub(crate) fn invited_by(&self) -> ProfileId {
        self.invited_by
    }

    pub(crate) fn invitee(&self) -> ProfileId {
        self.invitee
    }
}

/// The answer for a revoked invitation, whoever revoked it: a manager, a change to the inviter's
/// membership, or the accept-time check that the inviter could still issue it.
const INVITATION_REVOKED: &str = "invitation was revoked";

/// Parameters for creating an invitation.
pub struct CreateInvitationParams {
    pub invited_email: String,
    pub role: TeamRole,
}

/// Mint a 128-bit capability token, hex-encoded (32 chars). CSPRNG-backed —
/// NOT a UUID (which is time-sortable and guessable).
fn mint_token() -> String {
    let mut rng = UnwrapErr(SysRng);
    format!("{:016x}{:016x}", rng.next_u64(), rng.next_u64())
}

/// Create a pending invitation — the rank rule at invitation granularity (C16, R10): a manager
/// of the team may offer a role up to their own (an owner can invite at `owner`, a maintainer up
/// to `maintainer`), and a system admin any role; on the gating team only a system admin invites
/// (R11). The offered role is not checked against any membership row: an invitation is
/// email-keyed, and inviting a profile already on the team is allowed — their acceptance is a
/// no-op reporting their actual role (P12). A second pending invite for the same
/// `(team, email)` conflicts (partial unique index `idx_invitations_one_pending`).
pub async fn create_invitation(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    params: CreateInvitationParams,
) -> ApiResult<TeamInvitation> {
    // C16: the rank rule, reusing checks 1, 2 and 6 of `decide` (`authorize_invitation`).
    {
        let mut conn = pool.acquire().await?;
        MembershipAuthority::resolve(&mut conn, caller, team_id)
            .await?
            .authorize_invitation(params.role)?;
        // The team must exist and be live. Asked only AFTER the caller is known to manage it (S6),
        // so only a manager or a system admin learns the answer; without it a system admin's
        // invite to an unknown id surfaced as the insert's foreign-key failure (a 500).
        let live = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM kb_teams WHERE id = $1 AND is_active) AS "live!: bool""#,
            team_id
        )
        .fetch_one(&mut *conn)
        .await?;
        if !live {
            return Err(ApiError::NotFound(
                "team not found or not readable".to_string(),
            ));
        }
    }
    let caller = caller.profile_id();

    let id = Uuid::now_v7();
    let token = mint_token();

    // Expiry is enforced lazily — only `accept_invitation` ever flips a lapsed row to `expired`.
    // So a pending row can outlive its `expires_at` indefinitely while still occupying
    // `idx_invitations_one_pending`, which would 409 a legitimate re-invite of someone whose
    // prior invite is dead everywhere it is *displayed* (both list paths filter on
    // `expires_at > now()`). Sweep that lapsed-but-unswept row to `expired` first, in the same
    // transaction as the insert, so the partial index frees up. A still-live pending row
    // (`expires_at > now()`) is untouched and the insert correctly conflicts. Matched on the
    // exact `(team_id, invited_email)` the unique index keys on — never a lower()-broadened set,
    // which could expire a differently-cased invite that would not have collided.
    let mut tx = pool.begin().await?;
    sqlx::query!(
        r#"
        UPDATE kb_team_invitations
           SET status = 'expired'
         WHERE team_id = $1
           AND invited_email = $2
           AND status = 'pending'
           AND expires_at <= now()
        "#,
        team_id,
        params.invited_email,
    )
    .execute(&mut *tx)
    .await?;

    let row = sqlx::query_as!(
        TeamInvitation,
        r#"
        INSERT INTO kb_team_invitations
            (id, team_id, invited_email, invited_by_profile_id, role, token, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending')
        RETURNING id, team_id, invited_email, invited_by_profile_id,
                  role AS "role: TeamRole", token,
                  status AS "status: InvitationStatus", expires_at, created, revoked_at
        "#,
        id,
        team_id,
        params.invited_email,
        *caller,
        params.role as TeamRole,
        token,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            ApiError::Conflict("a pending invitation already exists for this email".to_string())
        }
        _ => ApiError::from(e),
    })?;
    tx.commit().await?;

    Ok(row)
}

/// Redeem an invitation token for `caller`, **only if `caller` is the person invited**: the
/// invitation must reach them through `vw_invitee_invitations`, the same predicate that lists it in
/// their inbox (a verified email exactly one profile owns). The token locates the invitation; it is
/// not the authority. A forwarded or leaked token redeems for nobody else, and a machine principal,
/// which holds no verified email, is never an invitee.
///
/// The claim is the authority at accept (P12): exactly one caller can hold it, so a revoke, a
/// decline or a second accept racing this one cannot land between the check and the write — the
/// invitation row's own lock serializes them, and the claim, the team lock, the member write and
/// the actual-role read are one transaction, so a crash leaves either an unclaimed invitation or
/// a joined member, never a claimed one with no row.
///
/// The invitation also redeems only while its **inviter could still issue it** (task
/// `01a14591`): the invitation gate is re-applied to the inviter under the team lock, before the
/// claim. An owner who has since been demoted, removed or lost their standing cannot have an
/// owner-level invitation redeemed after them; an invitation that fails the check is revoked and
/// answered as revoked. The check runs only for the invitee, so a token holder who is not the
/// invitee can neither set it off nor learn its answer. The team lock is taken before the
/// invitation's row lock, the order `membership_write` holds when it revokes invitations.
///
/// When nothing is claimed, the answer is read
/// from the row as it now stands: revoked, declined, expired (by the database clock, and flipped
/// to `expired`), a team that no longer exists, or — for a live pending invitation — not the
/// caller's (403, naming no address). An already-accepted invitation answers 200 to a caller
/// already on the team, 409 to anyone else.
pub async fn accept_invitation(
    pool: &PgPool,
    caller: &HumanPrincipal,
    token: &str,
) -> ApiResult<AcceptInvitationResponse> {
    let caller = caller.profile_id();
    let mut tx = pool.begin().await?;

    // Which team to lock, read unlocked: `membership_write` revokes invitation rows while
    // holding the team lock, so this path must take the team lock before any invitation row
    // lock, or the two deadlock. The read admits only a live invitation addressed to the caller,
    // so a token holder who is not the invitee (or a spent token) never takes the team lock and
    // goes straight to the refusal. Everything it assumes is read again under the locks.
    let team_id = sqlx::query_scalar!(
        r#"
        SELECT i.team_id
          FROM kb_team_invitations i
         WHERE i.token = $1
           AND i.status = 'pending'
           AND i.revoked_at IS NULL
           AND i.expires_at > now()
           AND EXISTS (SELECT 1 FROM vw_invitee_invitations v
                        WHERE v.id = i.id AND v.invitee_profile_id = $2)
        "#,
        token,
        *caller,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let claimed = match team_id {
        None => None,
        Some(team_id) => {
            team_membership::lock_team(&mut tx, team_id).await?;
            // The invitation under its own lock, and only if it is live and the caller's: a
            // token holder who is not the invitee goes no further, so cannot set off the check
            // below or learn its answer (they get the "addressed to someone else" refusal). A
            // team that is gone or soft-deleted reads as no row here (`vw_invitee_invitations`
            // reads live teams only) and is answered from the row as it stands.
            let live = sqlx::query!(
                r#"
                SELECT i.id, i.role AS "role: TeamRole", i.invited_by_profile_id
                  FROM kb_team_invitations i
                 WHERE i.token = $1
                   AND i.team_id = $2
                   AND i.status = 'pending'
                   AND i.revoked_at IS NULL
                   AND i.expires_at > now()
                   AND EXISTS (SELECT 1 FROM vw_invitee_invitations v
                                WHERE v.id = i.id AND v.invitee_profile_id = $3)
                   FOR UPDATE OF i
                "#,
                token,
                team_id,
                *caller,
            )
            .fetch_optional(&mut *tx)
            .await?;
            match live {
                None => None,
                Some(live) => {
                    // An invitation redeems only while its inviter could still issue it (task
                    // `01a14591`): the invitation gate, re-applied to the inviter as they now
                    // stand. A routed demotion or removal already revoked what it lowered
                    // (`membership_write`); this also covers what is not a routed membership
                    // write — the inviter's standing or governance, the derived writers, and an
                    // invitation created in a race with its inviter's demotion. One that fails it
                    // is revoked, and answered as any revoked invitation is.
                    let ceiling = team_membership::inviter_ceiling(
                        &mut tx,
                        team_id,
                        ProfileId::from(live.invited_by_profile_id),
                    )
                    .await?;
                    if !team_membership::may_offer(ceiling, live.role) {
                        sqlx::query!(
                            "UPDATE kb_team_invitations SET revoked_at = now() WHERE id = $1",
                            live.id,
                        )
                        .execute(&mut *tx)
                        .await?;
                        tx.commit().await?;
                        return Err(ApiError::BadRequest(INVITATION_REVOKED.to_string()));
                    }

                    // The claim: flip the invitation to `accepted` and read what it offered
                    // (team, role, inviter). The row is locked and was just read live and the
                    // caller's, so this cannot miss.
                    Some(
                        sqlx::query!(
                            r#"
                            UPDATE kb_team_invitations
                               SET status = 'accepted'
                             WHERE id = $1
                            RETURNING team_id, role AS "role: TeamRole", invited_by_profile_id
                            "#,
                            live.id,
                        )
                        .fetch_one(&mut *tx)
                        .await?,
                    )
                }
            }
        }
    };

    let joined = if let Some(row) = claimed {
        let claimed = ClaimedInvitation {
            team_id: row.team_id,
            role: row.role,
            invited_by: ProfileId::from(row.invited_by_profile_id),
            invitee: caller,
        };
        let team_id = claimed.team_id();
        // The member row under the team lock (P12), the same locks a member act holds, so the
        // join serializes with the roster's other recorded acts and is itself recorded.
        team_membership::locked(&mut tx, team_id, caller).await?;
        let change = team_membership::AuthorizedMembershipChange::from_claimed_invitation(claimed);
        team_membership::membership_write(&mut tx, None, change).await?;

        // The member's ACTUAL role: a fresh join lands at the invited role, but a profile already
        // on the team keeps their own — their acceptance is a no-op, and inventing the invited
        // role would tell them a rank they do not hold (P12's bite).
        let role = role_on_team_on(&mut tx, team_id, caller)
            .await?
            .ok_or_else(|| ApiError::Internal("a claimed invitation joined nobody".to_string()))?;
        let slug = sqlx::query_scalar!(
            r#"SELECT slug AS "slug!" FROM kb_teams WHERE id = $1"#,
            team_id
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Some((team_id, slug, role))
    } else {
        // Nothing was claimed: release the empty transaction and answer from the row as it stands.
        tx.rollback().await?;
        None
    };

    if let Some((team_id, team_slug, role)) = joined {
        return Ok(AcceptInvitationResponse {
            team_id,
            team_slug,
            role,
        });
    }

    let inv = unclaimed(pool, token).await?;
    match inv.status {
        InvitationStatus::Accepted => match role_on_team(pool, inv.team_id, caller).await? {
            Some(role) => Ok(AcceptInvitationResponse {
                team_id: inv.team_id,
                team_slug: inv.slug,
                role,
            }),
            None => Err(ApiError::Conflict(
                "invitation already redeemed".to_string(),
            )),
        },
        InvitationStatus::Declined => {
            Err(ApiError::BadRequest("invitation was declined".to_string()))
        }
        InvitationStatus::Expired => {
            Err(ApiError::BadRequest("invitation has expired".to_string()))
        }
        InvitationStatus::Pending => {
            pending_refusal(pool, &inv).await?;
            Err(ApiError::BadRequest("invitation has expired".to_string()))
        }
    }
}

/// Decline an invitation, **only as the person invited** (`vw_invitee_invitations`, as for
/// accept): a token holder who is not the invitee cannot spend someone else's invitation. One
/// guarded statement, as for accept. Idempotent if already declined; declining an accepted
/// invitation is a `BadRequest`. An expired invitation is already spent, so declining one changes
/// nothing (a pending row past its expiry is flipped to `expired`, as accept does).
pub async fn decline_invitation(
    pool: &PgPool,
    caller: &HumanPrincipal,
    token: &str,
) -> ApiResult<()> {
    let caller = caller.profile_id();
    let declined = sqlx::query_scalar!(
        r#"
        UPDATE kb_team_invitations i
           SET status = 'declined'
         WHERE i.token = $1
           AND i.status = 'pending'
           AND i.revoked_at IS NULL
           AND i.expires_at > now()
           AND EXISTS (SELECT 1 FROM vw_invitee_invitations v
                        WHERE v.id = i.id AND v.invitee_profile_id = $2)
        RETURNING i.id
        "#,
        token,
        *caller,
    )
    .fetch_optional(pool)
    .await?;
    if declined.is_some() {
        return Ok(());
    }

    let inv = unclaimed(pool, token).await?;
    match inv.status {
        InvitationStatus::Declined | InvitationStatus::Expired => Ok(()),
        InvitationStatus::Accepted => Err(ApiError::BadRequest(
            "invitation was already accepted".to_string(),
        )),
        InvitationStatus::Pending => pending_refusal(pool, &inv).await,
    }
}

/// An invitation the guarded claim did not take, as it stands now.
struct Unclaimed {
    id: Uuid,
    team_id: Uuid,
    slug: String,
    status: InvitationStatus,
    expired: bool,
    team_active: bool,
}

/// Read the invitation a claim did not take, answering 404 for an unknown token and 400 for a
/// revoked one. Revocation is authoritative and orthogonal to status (a revoked invite keeps
/// status = 'pending'), so it is answered before any status. Expiry is the database's clock, the
/// one the claim used.
async fn unclaimed(pool: &PgPool, token: &str) -> ApiResult<Unclaimed> {
    let row = sqlx::query!(
        r#"
        SELECT i.id, i.team_id, t.slug,
               i.status AS "status: InvitationStatus",
               i.revoked_at IS NOT NULL AS "revoked!",
               i.expires_at <= now() AS "expired!",
               t.is_active AS "team_active!"
          FROM kb_team_invitations i
          JOIN kb_teams t ON t.id = i.team_id
         WHERE i.token = $1
        "#,
        token,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ApiError::NotFound("invitation not found, already used, or expired".to_string())
    })?;
    if row.revoked {
        return Err(ApiError::BadRequest(INVITATION_REVOKED.to_string()));
    }
    Ok(Unclaimed {
        id: row.id,
        team_id: row.team_id,
        slug: row.slug,
        status: row.status,
        expired: row.expired,
        team_active: row.team_active,
    })
}

/// Why a still-pending invitation was not claimed. Past its expiry it is flipped to `expired` and
/// this returns `Ok` (the caller words the answer: accept refuses, decline is a no-op). A team that
/// no longer exists is its own answer. Otherwise the caller is not the invitee: 403, naming no
/// address — the holder learns only that the invitation is not theirs.
async fn pending_refusal(pool: &PgPool, inv: &Unclaimed) -> ApiResult<()> {
    if inv.expired {
        sqlx::query!(
            "UPDATE kb_team_invitations SET status = 'expired' WHERE id = $1 AND status = 'pending'",
            inv.id,
        )
        .execute(pool)
        .await?;
        return Ok(());
    }
    if !inv.team_active {
        return Err(ApiError::BadRequest(
            "the team this invitation is for no longer exists".to_string(),
        ));
    }
    Err(ApiError::ForbiddenDetail(
        "this invitation is addressed to someone else; it can only be used from the account \
         whose verified email it was sent to"
            .to_string(),
    ))
}

/// Revoke (withdraw) a pending invitation — the inviting team's counterpart to the invitee's
/// decline. Auth: any managing authority on the team (R10) — an owner- or maintainer-ranked
/// caller, or a system admin, may revoke any pending invitation, an owner-level one included.
///
/// Addressed by invitation `id` (the manager-facing handle returned by `list_invitations`), scoped
/// to `team_id` so a maintainer of team A can never revoke an invite belonging to team B by id
/// alone. Only a *live pending* invite can be revoked: an already-`accepted` invite means the
/// invitee is a member (remove them via the membership surface), `declined`/`expired` are already
/// terminal, and re-revoking an already-revoked invite is a no-op error. Revocation is modelled
/// as a `revoked_at` timestamp (not a status value) — see the migration for why. The guard and the
/// write are one statement, so there is no check-then-act gap.
pub async fn revoke_invitation(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    invitation_id: Uuid,
) -> ApiResult<()> {
    // R10: the gate is the authority, not a role — on the gating team an owner or maintainer row
    // confers none (R11), and a system admin who is not a member may revoke.
    {
        let mut conn = pool.acquire().await?;
        if !MembershipAuthority::resolve(&mut conn, caller, team_id)
            .await?
            .manages()
        {
            return Err(ApiError::Forbidden);
        }
    }

    let revoked = sqlx::query_scalar!(
        r#"
        UPDATE kb_team_invitations
           SET revoked_at = now()
         WHERE id = $1
           AND team_id = $2
           AND status = 'pending'
           AND revoked_at IS NULL
        RETURNING id
        "#,
        invitation_id,
        team_id,
    )
    .fetch_optional(pool)
    .await?;

    if revoked.is_some() {
        return Ok(());
    }

    // Nothing was revoked — disambiguate "no such invite here" from "not in a revocable state"
    // (already accepted/declined/expired, or already revoked) so the caller gets an actionable
    // error rather than an opaque 404.
    let exists = sqlx::query_scalar!(
        r#"SELECT 1 AS "one!" FROM kb_team_invitations WHERE id = $1 AND team_id = $2"#,
        invitation_id,
        team_id,
    )
    .fetch_optional(pool)
    .await?;

    match exists {
        None => Err(ApiError::NotFound(
            "invitation not found for this team".to_string(),
        )),
        Some(_) => Err(ApiError::BadRequest(
            "only a live pending invitation can be revoked".to_string(),
        )),
    }
}

/// List pending, non-expired invitations for a team. Auth: any managing authority (R10) —
/// the same bar as revoke.
pub async fn list_invitations(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
) -> ApiResult<Vec<TeamInvitation>> {
    {
        let mut conn = pool.acquire().await?;
        if !MembershipAuthority::resolve(&mut conn, caller, team_id)
            .await?
            .manages()
        {
            return Err(ApiError::Forbidden);
        }
    }
    let rows = sqlx::query_as!(
        TeamInvitation,
        r#"
        SELECT id, team_id, invited_email, invited_by_profile_id,
               role AS "role: TeamRole", token,
               status AS "status: InvitationStatus", expires_at, created, revoked_at
          FROM kb_team_invitations
         WHERE team_id = $1 AND status = 'pending' AND expires_at > now()
           AND revoked_at IS NULL
         ORDER BY created DESC
        "#,
        team_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List a caller's own pending, non-expired invitations. Resolution matches
/// `invited_email` (case-insensitively) against the caller's VERIFIED
/// `kb_profile_auth_links` emails (Option A — an unverified link email is an
/// attacker-controllable claim and must not resolve someone else's invite),
/// still guarded so an email whose verified holders span more than one profile
/// is discounted (never leaks another profile's invite, and since redemption reads the same
/// predicate, nobody can redeem it either: the duplicate address must be resolved).
/// Restricting the ambiguity count to verified links also means an unverified squatter row
/// cannot suppress the legitimate holder's invites.
/// Auth: any authenticated caller (their own invitations only).
pub async fn list_for_profile(
    pool: &PgPool,
    caller: &HumanPrincipal,
) -> ApiResult<Vec<InviteeInvitation>> {
    let caller = caller.profile_id();
    let rows = sqlx::query_as!(
        InviteeInvitation,
        r#"
        SELECT id as "id!", team_id as "team_id!",
               team_slug as "team_slug!", team_name as "team_name!",
               invited_email as "invited_email!", invited_by_profile_id as "invited_by_profile_id!",
               role as "role!: TeamRole", token as "token!",
               status as "status!: InvitationStatus",
               expires_at as "expires_at!", created as "created!"
          FROM vw_invitee_invitations
         WHERE invitee_profile_id = $1
         ORDER BY created DESC
        "#,
        *caller,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// How many invitations are waiting on the caller — **without handing them over**.
///
/// The same set [`list_for_profile`] returns, counted in the database instead of in the client.
/// That is the whole point: `temper warmup` runs from the `SessionStart` hook and wants an
/// integer, and the rows it was fetching to get one each carry a redemption `token`. Counting
/// here means no redemption token crosses the wire to answer "how many?".
///
/// Both read `vw_invitee_invitations`, so the five clauses that decide what "waiting on me"
/// means are stated once. A second inline copy would not fail loudly when it drifted — it would
/// report a different number from the command it tells the reader to run.
///
/// `team_slug` asks a second question in the same round trip: *of those, how many are to this
/// team?* `None` in, `matching: None` out — the caller had no team to ask about, and that is a
/// different fact from asking and being told none. `temper warmup` is what needs the difference:
/// it may say an invitation grants the context it could not read only when it has established
/// that one does.
pub async fn count_for_profile(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_slug: Option<&str>,
) -> ApiResult<PendingInvitationCounts> {
    let caller = caller.profile_id();
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*)::int4 as "count!",
               (COUNT(*) FILTER (WHERE team_slug = $2))::int4 as "matching!"
          FROM vw_invitee_invitations
         WHERE invitee_profile_id = $1
        "#,
        *caller,
        team_slug,
    )
    .fetch_one(pool)
    .await?;

    Ok(PendingInvitationCounts {
        count: row.count,
        // `Some` only when a team was named. The FILTER yields 0 for a NULL parameter, and
        // returning that 0 would claim the caller holds no invitation to a team nobody asked
        // about.
        matching: team_slug.map(|_| row.matching),
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::types::ids::ProfileId;
    use temper_core::types::team::TeamRole;

    /// Insert a profile with the given handle, return its ProfileId. It gets the `<handle>@web`
    /// emitter entity the first authenticated request would provision, so an accept it performs
    /// can be recorded.
    async fn mk_profile(pool: &PgPool, handle: &str) -> ProfileId {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap();
        // Approved, as every caller who reaches a service is: an invitation redeems only while
        // its inviter could still issue it, and a profile without standing can issue nothing.
        crate::test_support::approve(pool, id).await;
        sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2 || '@web')")
            .bind(id)
            .bind(handle)
            .execute(pool)
            .await
            .unwrap();
        ProfileId::from(id)
    }

    /// Insert a root team with the given slug, return its id.
    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_teams (id, slug, name) VALUES (gen_random_uuid(), $1, $1) RETURNING id",
        )
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn add_member(pool: &PgPool, team: Uuid, profile: ProfileId, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
             VALUES ($1, $2, $3::team_role, 'native'::team_member_source)",
        )
        .bind(team)
        .bind(*profile)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seed a root team with an owner; return (team_id, owner).
    async fn seed_team_with_owner(pool: &PgPool) -> (Uuid, ProfileId) {
        let owner = mk_profile(pool, "owner").await;
        let team = mk_team(pool, "acme").await;
        add_member(pool, team, owner, "owner").await;
        (team, owner)
    }

    /// Attach an auth-link email to a profile — the resolver matches on these
    /// (verified links only, so the standard fixture inserts verified).
    async fn add_auth_email(
        pool: &PgPool,
        profile: ProfileId,
        provider_uid: &str,
        email: Option<&str>,
    ) {
        add_auth_email_with_verified(pool, profile, provider_uid, email, true).await;
    }

    /// [`add_auth_email`] with explicit verification — for the unverified-link cases.
    async fn add_auth_email_with_verified(
        pool: &PgPool,
        profile: ProfileId,
        provider_uid: &str,
        email: Option<&str>,
        verified: bool,
    ) {
        sqlx::query(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default) \
             VALUES (gen_random_uuid(), $1, 'test', $2, $3, $4, true)",
        )
        .bind(*profile)
        .bind(provider_uid)
        .bind(email)
        .bind(verified)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert an invitation with an explicit status and expiry offset (days; negative = expired).
    async fn seed_invite(
        pool: &PgPool,
        team: Uuid,
        email: &str,
        by: ProfileId,
        status: &str,
        expires_days: i32,
    ) {
        sqlx::query(
            "INSERT INTO kb_team_invitations \
               (id, team_id, invited_email, invited_by_profile_id, role, token, status, expires_at) \
             VALUES (gen_random_uuid(), $1, $2, $3, 'member'::team_role, $4, \
                     $5::invitation_status, now() + make_interval(days => $6))",
        )
        .bind(team)
        .bind(email)
        .bind(*by)
        .bind(format!("tok-{}", Uuid::now_v7()))
        .bind(status)
        .bind(expires_days)
        .execute(pool)
        .await
        .unwrap();
    }

    /// A team made the gating team.
    async fn mk_gating_team(pool: &PgPool) -> Uuid {
        let team = mk_team(pool, "gate").await;
        sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'gate'")
            .execute(pool)
            .await
            .unwrap();
        team
    }

    /// The `team_membership_changed` payloads whose subject is `team`, oldest first.
    async fn membership_events(pool: &PgPool, team: Uuid) -> Vec<serde_json::Value> {
        sqlx::query_scalar(
            "SELECT e.payload FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'team_membership_changed' \
                AND e.\"references\" @> jsonb_build_array(jsonb_build_object( \
                      'rel', 'subject', \
                      'target', jsonb_build_object('kind', 'kb_teams', 'id', $1::uuid))) \
              ORDER BY e.id",
        )
        .bind(team)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_resolves_unambiguous_email_only(pool: PgPool) {
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("invitee@x.com")).await;
        let team = mk_team(&pool, "platform").await;
        add_member(&pool, team, inviter, "owner").await;
        seed_invite(&pool, team, "invitee@x.com", inviter, "pending", 7).await;

        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
        )
        .await
        .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].team_slug, "platform");
        assert_eq!(got[0].invited_email, "invitee@x.com");
        assert!(!got[0].token.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_case_insensitive_match(pool: PgPool) {
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("invitee@x.com")).await;
        let team = mk_team(&pool, "platform").await;
        add_member(&pool, team, inviter, "owner").await;
        seed_invite(&pool, team, "Invitee@X.com", inviter, "pending", 7).await; // mixed case

        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
        )
        .await
        .unwrap();
        assert_eq!(got.len(), 1);
    }

    /// A look-alike is not the invited address: an invitation to `kate@x.com` is not addressed to
    /// the verified holder of the U+212A KELVIN SIGN spelling, which the database collation's
    /// `lower()` would fold to `k`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_does_not_fold_look_alikes(pool: PgPool) {
        let inviter = mk_profile(&pool, "inviter").await;
        let holder = mk_profile(&pool, "holder").await;
        add_auth_email(&pool, holder, "holder-uid", Some("\u{212A}ate@x.com")).await;
        let team = mk_team(&pool, "platform").await;
        add_member(&pool, team, inviter, "owner").await;
        seed_invite(&pool, team, "kate@x.com", inviter, "pending", 7).await;

        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *holder).await,
        )
        .await
        .unwrap();
        assert!(
            got.is_empty(),
            "a look-alike address is not the invited one"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_discounts_ambiguous_email(pool: PgPool) {
        // Two profiles both hold dup@x.com — ambiguous, must be discounted for the caller.
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("dup@x.com")).await;
        let other = mk_profile(&pool, "other").await;
        add_auth_email(&pool, other, "other-uid", Some("dup@x.com")).await;
        let team = mk_team(&pool, "platform").await;
        add_member(&pool, team, inviter, "owner").await;
        seed_invite(&pool, team, "dup@x.com", inviter, "pending", 7).await;

        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
        )
        .await
        .unwrap();
        assert!(got.is_empty(), "ambiguous email must not resolve");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_excludes_declined_expired_softdeleted(pool: PgPool) {
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("invitee@x.com")).await;
        let team = mk_team(&pool, "platform").await;
        add_member(&pool, team, inviter, "owner").await;

        seed_invite(&pool, team, "invitee@x.com", inviter, "declined", 7).await;
        seed_invite(&pool, team, "invitee@x.com", inviter, "pending", -1).await; // expired
        let dead = mk_team(&pool, "dead").await;
        add_member(&pool, dead, inviter, "owner").await;
        sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
            .bind(dead)
            .execute(&pool)
            .await
            .unwrap();
        seed_invite(&pool, dead, "invitee@x.com", inviter, "pending", 7).await;

        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
        )
        .await
        .unwrap();
        assert!(got.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_unverified_email_never_matches(pool: PgPool) {
        // The caller's link holds the invited email but UNVERIFIED — an
        // unverified email is an attacker-controllable claim, so it must not
        // resolve the invite (Option A).
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email_with_verified(&pool, invitee, "invitee-uid", Some("invitee@x.com"), false)
            .await;
        let team = mk_team(&pool, "platform").await;
        add_member(&pool, team, inviter, "owner").await;
        seed_invite(&pool, team, "invitee@x.com", inviter, "pending", 7).await;

        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
        )
        .await
        .unwrap();
        assert!(got.is_empty(), "unverified email must not resolve");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_unverified_squatter_does_not_suppress(pool: PgPool) {
        // Another profile holds the same email UNVERIFIED — the ambiguity
        // guard counts verified holders only, so the legitimate (verified)
        // holder still resolves; the squatter cannot deny service.
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("shared@x.com")).await;
        let squatter = mk_profile(&pool, "squatter").await;
        add_auth_email_with_verified(&pool, squatter, "squatter-uid", Some("shared@x.com"), false)
            .await;
        let team = mk_team(&pool, "platform").await;
        add_member(&pool, team, inviter, "owner").await;
        seed_invite(&pool, team, "shared@x.com", inviter, "pending", 7).await;

        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
        )
        .await
        .unwrap();
        assert_eq!(got.len(), 1, "verified holder resolves despite squatter");
        let denied = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *squatter).await,
        )
        .await
        .unwrap();
        assert!(
            denied.is_empty(),
            "squatter's unverified link matches nothing"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_for_profile_null_email_caller_empty(pool: PgPool) {
        let agent = mk_profile(&pool, "agent").await;
        add_auth_email(&pool, agent, "agent-uid", None).await;
        let got = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *agent).await,
        )
        .await
        .unwrap();
        assert!(got.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_by_owner_succeeds(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "alice@example.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner can invite");
        assert_eq!(inv.invited_email, "alice@example.com");
        assert_eq!(inv.role, TeamRole::Member);
        assert_eq!(inv.status, InvitationStatus::Pending);
        assert_eq!(inv.token.len(), 32); // 16 bytes hex
    }

    /// C16, bite: an owner CAN offer `owner` — the cap is the inviter's own rank, so the
    /// blanket owner refusal is gone. On the pre-rank code this was refused outright.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_owner_invites_at_owner(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "co-owner@e.com".into(),
                role: TeamRole::Owner,
            },
        )
        .await
        .expect("an owner may offer owner");
        assert_eq!(inv.role, TeamRole::Owner);
    }

    /// C16, pin: a maintainer inviting at `owner` is refused — above the inviter's own rank —
    /// and no invitation row is created.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_maintainer_inviting_at_owner_is_refused(pool: PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let mt = mk_profile(&pool, "mt").await;
        add_member(&pool, team_id, mt, "maintainer").await;
        let err = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *mt).await,
            team_id,
            CreateInvitationParams {
                invited_email: "a@e.com".into(),
                role: TeamRole::Owner,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_team_invitations WHERE team_id = $1")
                .bind(team_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(pending, 0);
    }

    /// C16/R10, bite: a maintainer may revoke an owner's owner-level invitation. No
    /// owner-level invitation can exist on the pre-rank code, so the whole scenario —
    /// the creation the revocation stands on — is what the bite bites on.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_maintainer_revokes_an_owners_owner_level_invitation(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let mt = mk_profile(&pool, "mt").await;
        add_member(&pool, team_id, mt, "maintainer").await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "co@e.com".into(),
                role: TeamRole::Owner,
            },
        )
        .await
        .expect("an owner may offer owner");

        revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *mt).await,
            team_id,
            inv.id,
        )
        .await
        .expect("a maintainer revokes an owner-level invitation");
        // Revocation is `revoked_at`, not a status: the status stays `pending`.
        let revoked: bool = sqlx::query_scalar(
            "SELECT revoked_at IS NOT NULL FROM kb_team_invitations WHERE id = $1",
        )
        .bind(inv.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(revoked, "the owner-level invitation is withdrawn");
    }

    /// C16/R10/R11, bite: a system admin who is not a member manages any team's invitations,
    /// the gating team's included — creates one at `owner`, lists it, revokes it — and is the
    /// one caller told a team does not exist (a 404 rather than the insert's foreign-key 500).
    /// On the pre-rank code a non-member admin was refused all three with a 403.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_non_member_system_admin_manages_any_teams_invitations(pool: PgPool) {
        let gate = mk_gating_team(&pool).await;
        let admin = mk_profile(&pool, "admin").await;
        crate::test_support::approved_admin(&pool, *admin).await;
        let caller = crate::test_support::human_principal_for(&pool, *admin).await;

        for team in [gate, mk_team(&pool, "plain").await] {
            let inv = create_invitation(
                &pool,
                &caller,
                team,
                CreateInvitationParams {
                    invited_email: "new@e.com".into(),
                    role: TeamRole::Owner,
                },
            )
            .await
            .expect("an admin may offer any role on any team");
            let listed = list_invitations(&pool, &caller, team)
                .await
                .expect("an admin lists");
            assert!(listed.iter().any(|i| i.id == inv.id), "{listed:?}");
            revoke_invitation(&pool, &caller, team, inv.id)
                .await
                .expect("an admin revokes");
        }

        let err = create_invitation(
            &pool,
            &caller,
            Uuid::now_v7(),
            CreateInvitationParams {
                invited_email: "nobody@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::NotFound(_)), "got {err:?}");
    }

    /// Create an invitation from `by` at `role`, through the real gate.
    async fn invite_at(
        pool: &PgPool,
        team: Uuid,
        by: ProfileId,
        email: &str,
        role: TeamRole,
    ) -> TeamInvitation {
        create_invitation(
            pool,
            &crate::test_support::human_principal_for(pool, *by).await,
            team,
            CreateInvitationParams {
                invited_email: email.into(),
                role,
            },
        )
        .await
        .expect("the inviter may offer this role")
    }

    async fn is_revoked(pool: &PgPool, id: Uuid) -> bool {
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM kb_team_invitations WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// A profile with a verified address, ready to accept what is sent to it.
    async fn invitee(pool: &PgPool, handle: &str, email: &str) -> ProfileId {
        let p = mk_profile(pool, handle).await;
        add_auth_email(pool, p, &format!("{handle}-uid"), Some(email)).await;
        p
    }

    async fn accept_as(
        pool: &PgPool,
        who: ProfileId,
        token: &str,
    ) -> ApiResult<AcceptInvitationResponse> {
        accept_invitation(
            pool,
            &crate::test_support::human_principal_for(pool, *who).await,
            token,
        )
        .await
    }

    /// Task 01a14591, bite: an owner offers `owner`, then hands the team over and lowers
    /// themselves to `maintainer`. The offer is above what they can now make, so the lowering
    /// revokes it, and it no longer redeems. Before, it redeemed: a co-owner minted by someone who
    /// had stopped being one, removable afterwards only by a system admin (R6).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_owner_invitation_is_revoked_when_its_inviter_steps_down(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let joiner = invitee(&pool, "invitee", "i@e.com").await;
        let inv = invite_at(&pool, team_id, owner, "i@e.com", TeamRole::Owner).await;

        let co = mk_profile(&pool, "co").await;
        add_member(&pool, team_id, co, "owner").await;
        crate::services::team_service::change_role(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            *owner,
            TeamRole::Maintainer,
        )
        .await
        .expect("an owner lowers their own role");

        assert!(
            is_revoked(&pool, inv.id).await,
            "the lowering revokes the offer"
        );
        let err = accept_as(&pool, joiner, &inv.token).await.unwrap_err();
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == INVITATION_REVOKED),
            "got {err:?}"
        );
        assert_eq!(role_on_team(&pool, team_id, joiner).await.unwrap(), None);
    }

    /// Task 01a14591, pin: an offer the inviter can still make stands. The owner offered
    /// `maintainer` and is now a maintainer; the invitation redeems at `maintainer`. (This is what
    /// `acceptance_after_the_inviters_demotion_still_redeems` pinned, narrowed to the offers
    /// within the inviter's new authority.)
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_invitation_within_the_inviters_new_role_still_redeems(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let joiner = invitee(&pool, "invitee", "i@e.com").await;
        let inv = invite_at(&pool, team_id, owner, "i@e.com", TeamRole::Maintainer).await;

        let co = mk_profile(&pool, "co").await;
        add_member(&pool, team_id, co, "owner").await;
        crate::services::team_service::change_role(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            *owner,
            TeamRole::Maintainer,
        )
        .await
        .expect("an owner lowers their own role");

        assert!(!is_revoked(&pool, inv.id).await);
        let resp = accept_as(&pool, joiner, &inv.token)
            .await
            .expect("an offer at the inviter's own role stands");
        assert_eq!(resp.role, TeamRole::Maintainer);
    }

    /// Task 01a14591, bite: a maintainer demoted to `member` can invite no one, so every offer
    /// they have pending goes — the invitation gate re-applied, not just "above their new role".
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_maintainer_demoted_to_member_keeps_no_pending_invitation(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let maint = mk_profile(&pool, "maint").await;
        add_member(&pool, team_id, maint, "maintainer").await;
        let at_member = invite_at(&pool, team_id, maint, "a@e.com", TeamRole::Member).await;
        let at_watcher = invite_at(&pool, team_id, maint, "b@e.com", TeamRole::Watcher).await;

        crate::services::team_service::change_role(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            *maint,
            TeamRole::Member,
        )
        .await
        .expect("an owner demotes a maintainer");

        assert!(is_revoked(&pool, at_member.id).await);
        assert!(is_revoked(&pool, at_watcher.id).await);
    }

    /// Task 01a14591, bite: an owner who leaves takes every pending invitation they issued on the
    /// team with them; another member's invitations stay.
    #[sqlx::test(migrations = "../../migrations")]
    async fn leaving_a_team_revokes_every_invitation_you_sent_it(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let co = mk_profile(&pool, "co").await;
        add_member(&pool, team_id, co, "owner").await;
        let mine_owner = invite_at(&pool, team_id, owner, "a@e.com", TeamRole::Owner).await;
        let mine_member = invite_at(&pool, team_id, owner, "b@e.com", TeamRole::Member).await;
        let theirs = invite_at(&pool, team_id, co, "c@e.com", TeamRole::Owner).await;

        crate::services::team_service::remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            *owner,
        )
        .await
        .expect("an owner with a co-owner may leave");

        assert!(is_revoked(&pool, mine_owner.id).await);
        assert!(is_revoked(&pool, mine_member.id).await);
        assert!(
            !is_revoked(&pool, theirs.id).await,
            "only the leaver's invitations go"
        );
    }

    /// Task 01a14591, pin: a system admin's invitation rests on their governance, not on any
    /// team row, so nothing done to the team's members touches it — not even a change to the
    /// admin's own row there.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_admins_invitation_is_untouched_by_membership_changes(pool: PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let admin = mk_profile(&pool, "admin").await;
        crate::test_support::approved_admin(&pool, *admin).await;
        add_member(&pool, team_id, admin, "owner").await;
        let joiner = invitee(&pool, "invitee", "i@e.com").await;
        let inv = invite_at(&pool, team_id, admin, "i@e.com", TeamRole::Owner).await;

        // Another member is demoted, and the admin lowers their own row.
        let maint = mk_profile(&pool, "maint").await;
        add_member(&pool, team_id, maint, "maintainer").await;
        let admin_p = crate::test_support::human_principal_for(&pool, *admin).await;
        crate::services::team_service::change_role(
            &pool,
            &admin_p,
            team_id,
            *maint,
            TeamRole::Member,
        )
        .await
        .expect("an admin demotes a maintainer");
        crate::services::team_service::change_role(
            &pool,
            &admin_p,
            team_id,
            *admin,
            TeamRole::Member,
        )
        .await
        .expect("an admin lowers their own row");
        assert!(!is_revoked(&pool, inv.id).await);
        let resp = accept_as(&pool, joiner, &inv.token)
            .await
            .expect("an admin's offer stands");
        assert_eq!(resp.role, TeamRole::Owner);
    }

    /// Task 01a14591, bite at redemption: losing governance is not a membership write, so no
    /// revocation runs when it happens. The accept-time check catches it: the former admin can
    /// no longer offer `owner` on a team they are not on, so the invitation is revoked and
    /// refused.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_invitation_does_not_redeem_once_its_inviter_is_no_longer_an_admin(pool: PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let admin = mk_profile(&pool, "admin").await;
        crate::test_support::approved_admin(&pool, *admin).await;
        let joiner = invitee(&pool, "invitee", "i@e.com").await;
        let inv = invite_at(&pool, team_id, admin, "i@e.com", TeamRole::Owner).await;

        sqlx::query("DELETE FROM kb_principal_governance WHERE profile_id = $1")
            .bind(*admin)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !is_revoked(&pool, inv.id).await,
            "nothing ran at the demotion"
        );

        let err = accept_as(&pool, joiner, &inv.token).await.unwrap_err();
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == INVITATION_REVOKED),
            "got {err:?}"
        );
        assert!(is_revoked(&pool, inv.id).await, "the refusal revokes it");
        assert_eq!(role_on_team(&pool, team_id, joiner).await.unwrap(), None);
        // And it stays refused, as any revoked invitation is.
        let again = accept_as(&pool, joiner, &inv.token).await.unwrap_err();
        assert!(matches!(&again, ApiError::BadRequest(m) if m == INVITATION_REVOKED));
    }

    /// Task 01a14591, bite at redemption: an inviter whose standing is revoked can act on
    /// nothing, so their pending invitations — at any role — no longer redeem, though their team
    /// row is untouched.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_invitation_does_not_redeem_once_its_inviter_has_lost_standing(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let joiner = invitee(&pool, "invitee", "i@e.com").await;
        let inv = invite_at(&pool, team_id, owner, "i@e.com", TeamRole::Watcher).await;

        sqlx::query("UPDATE kb_principal_standing SET state = 'revoked' WHERE profile_id = $1")
            .bind(*owner)
            .execute(&pool)
            .await
            .unwrap();

        let err = accept_as(&pool, joiner, &inv.token).await.unwrap_err();
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == INVITATION_REVOKED),
            "got {err:?}"
        );
        assert!(is_revoked(&pool, inv.id).await, "the refusal revokes it");
        assert_eq!(role_on_team(&pool, team_id, joiner).await.unwrap(), None);
    }

    /// Task 01a14591, bite at redemption on the ROLE half of the check: a demotion that did
    /// not go through `membership_write` (a derived writer, such as the machine-cap trigger,
    /// writes the row directly) revoked nothing, so the accept-time check alone refuses the
    /// owner-level offer of someone who is now a maintainer.
    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_refuses_an_offer_above_an_inviter_demoted_outside_the_writer(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let joiner = invitee(&pool, "invitee", "i@e.com").await;
        let at_owner = invite_at(&pool, team_id, owner, "i@e.com", TeamRole::Owner).await;
        let at_maint = invite_at(&pool, team_id, owner, "j@e.com", TeamRole::Maintainer).await;
        let other = invitee(&pool, "other", "j@e.com").await;

        sqlx::query(
            "UPDATE kb_team_members SET role = 'maintainer' WHERE team_id = $1 AND profile_id = $2",
        )
        .bind(team_id)
        .bind(*owner)
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            !is_revoked(&pool, at_owner.id).await,
            "nothing ran at the demotion"
        );

        let err = accept_as(&pool, joiner, &at_owner.token).await.unwrap_err();
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == INVITATION_REVOKED),
            "got {err:?}"
        );
        assert!(is_revoked(&pool, at_owner.id).await);
        let resp = accept_as(&pool, other, &at_maint.token)
            .await
            .expect("an offer at the inviter's new role still redeems");
        assert_eq!(resp.role, TeamRole::Maintainer);
    }

    /// Task 01a14591, bite: a manager removing a member revokes the removed member's invitations,
    /// as a self-leave does.
    #[sqlx::test(migrations = "../../migrations")]
    async fn removing_a_maintainer_revokes_their_invitations(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let maint = mk_profile(&pool, "maint").await;
        add_member(&pool, team_id, maint, "maintainer").await;
        let theirs = invite_at(&pool, team_id, maint, "a@e.com", TeamRole::Member).await;
        let mine = invite_at(&pool, team_id, owner, "b@e.com", TeamRole::Member).await;

        crate::services::team_service::remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            *maint,
        )
        .await
        .expect("an owner removes a maintainer");

        assert!(is_revoked(&pool, theirs.id).await);
        assert!(
            !is_revoked(&pool, mine.id).await,
            "the remover's own are untouched"
        );
    }

    /// A token holder who is not the invitee never takes the team lock: with the team row held
    /// by another transaction (a membership write in flight), their accept still returns its
    /// refusal at once instead of queueing behind it. Before the lookup was bound to the
    /// invitee, any holder of a forwarded token could make the accept path wait on, and then
    /// hold, the team lock that every membership writer takes.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_stranger_holding_the_token_never_waits_on_the_team_lock(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let _joiner = invitee(&pool, "invitee", "i@e.com").await;
        let stranger = invitee(&pool, "stranger", "s@e.com").await;
        let inv = invite_at(&pool, team_id, owner, "i@e.com", TeamRole::Member).await;

        let mut holder = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
            .bind(team_id)
            .execute(&mut *holder)
            .await
            .unwrap();

        let answered = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            accept_as(&pool, stranger, &inv.token),
        )
        .await
        .expect("a stranger's accept must not wait on the team lock");
        let err = answered.unwrap_err();
        assert!(matches!(err, ApiError::ForbiddenDetail(_)), "got {err:?}");
        holder.rollback().await.unwrap();
    }

    /// Task 01a14591, pin: the accept-time check runs only for the invitee. A token holder who is
    /// not the invitee gets the "addressed to someone else" refusal, learns nothing about the
    /// inviter, and leaves the invitation as it was; the invitee's own attempt is what revokes it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_stranger_holding_the_token_cannot_set_off_the_inviter_check(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let joiner = invitee(&pool, "invitee", "i@e.com").await;
        let stranger = invitee(&pool, "stranger", "s@e.com").await;
        let inv = invite_at(&pool, team_id, owner, "i@e.com", TeamRole::Member).await;
        sqlx::query("UPDATE kb_principal_standing SET state = 'revoked' WHERE profile_id = $1")
            .bind(*owner)
            .execute(&pool)
            .await
            .unwrap();

        let err = accept_as(&pool, stranger, &inv.token).await.unwrap_err();
        assert!(matches!(err, ApiError::ForbiddenDetail(_)), "got {err:?}");
        assert!(
            !is_revoked(&pool, inv.id).await,
            "a stranger's attempt changes nothing"
        );

        let err = accept_as(&pool, joiner, &inv.token).await.unwrap_err();
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == INVITATION_REVOKED),
            "got {err:?}"
        );
        assert!(is_revoked(&pool, inv.id).await);
    }

    /// P12, bite: a profile already on the team who accepts an owner invitation joins nothing
    /// and is told their ACTUAL role — not the role the invitation offered.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_existing_member_accepting_an_owner_invitation_is_told_their_actual_role(
        pool: PgPool,
    ) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let member = mk_profile(&pool, "member").await;
        add_auth_email(&pool, member, "member-uid", Some("m@e.com")).await;
        add_member(&pool, team_id, member, "member").await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "m@e.com".into(),
                role: TeamRole::Owner,
            },
        )
        .await
        .expect("an owner may offer owner");

        let resp = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *member).await,
            &inv.token,
        )
        .await
        .expect("an existing member's acceptance is a no-op");
        assert_eq!(
            resp.role,
            TeamRole::Member,
            "the actual role, not the offer"
        );
        assert_eq!(
            role_on_team(&pool, team_id, member).await.unwrap(),
            Some(TeamRole::Member)
        );
        // A no-op records nothing (C8): the fixture row predates the ledger, and the acceptance
        // added no event.
        assert!(
            membership_events(&pool, team_id).await.is_empty(),
            "a no-op acceptance records nothing"
        );
    }

    /// C14/R11, bite: a non-admin holding an owner row on the gating team invites nobody —
    /// on the pre-rank code `can_manage` admitted them.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_non_admin_owners_gating_team_invitation_is_refused(pool: PgPool) {
        let team = mk_gating_team(&pool).await;
        let owner = mk_profile(&pool, "owner").await;
        add_member(&pool, team, owner, "owner").await;
        let err = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team,
            CreateInvitationParams {
                invited_email: "a@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_team_invitations WHERE team_id = $1")
                .bind(team)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(pending, 0);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_non_manager_forbidden(pool: PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let stranger = mk_profile(&pool, "stranger").await;
        let err = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *stranger).await,
            team_id,
            CreateInvitationParams {
                invited_email: "a@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_duplicate_pending_conflicts(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let p = || CreateInvitationParams {
            invited_email: "dup@e.com".into(),
            role: TeamRole::Member,
        };
        create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            p(),
        )
        .await
        .unwrap();
        let err = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            p(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn fresh_invite_succeeds_after_decline(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("again@e.com")).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "again@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        decline_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .unwrap();
        // A new pending invite for the same (team, email) is now allowed.
        create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "again@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("re-invite after decline");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reinvite_succeeds_after_expiry_even_when_unswept(pool: PgPool) {
        // A lapsed invite whose status is still `pending` (nothing swept it) must not block a
        // re-invite. `create_invitation` sweeps it to `expired` first, freeing the partial index.
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        seed_invite(&pool, team_id, "lapsed@e.com", owner, "pending", -1).await;

        let fresh = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "lapsed@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("re-invite after (unswept) expiry");
        assert_eq!(fresh.status, InvitationStatus::Pending);

        // The lapsed row was flipped to `expired`; exactly one live pending row remains.
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_team_invitations \
             WHERE team_id=$1 AND invited_email=$2 AND status='pending'",
        )
        .bind(team_id)
        .bind("lapsed@e.com")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            pending, 1,
            "the old pending row must have been swept to expired"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reinvite_still_conflicts_while_a_live_pending_invite_exists(pool: PgPool) {
        // The sweep must only touch *lapsed* rows — a still-live pending invite must still 409.
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        seed_invite(&pool, team_id, "live@e.com", owner, "pending", 7).await;
        let err = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "live@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn revoke_pending_then_reinvite_succeeds(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "gone@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();

        revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            inv.id,
        )
        .await
        .expect("owner may revoke a pending invite");

        // Revocation is recorded as revoked_at (status stays 'pending'); a re-revoke is a no-op error.
        let revoked_at: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar(r#"SELECT revoked_at FROM kb_team_invitations WHERE id = $1"#)
                .bind(inv.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(revoked_at.is_some(), "revoke must stamp revoked_at");
        let reraise = revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            inv.id,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(reraise, ApiError::BadRequest(_)),
            "re-revoke is rejected"
        );

        // A revoked invite is excluded from the pending index, so a fresh invite for the same
        // (team, email) is allowed.
        create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "gone@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("re-invite after revoke");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn revoke_requires_manager(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "x@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        let stranger = mk_profile(&pool, "stranger").await;
        let err = revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *stranger).await,
            team_id,
            inv.id,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn revoke_unknown_invitation_is_not_found(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let err = revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            Uuid::now_v7(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn revoke_non_pending_is_bad_request(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("acc@e.com")).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "acc@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .unwrap();
        let err = revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            inv.id,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn revoked_invite_is_hidden_from_team_list(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "hide@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            list_invitations(
                &pool,
                &crate::test_support::human_principal_for(&pool, *owner).await,
                team_id
            )
            .await
            .unwrap()
            .len(),
            1
        );
        revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            inv.id,
        )
        .await
        .unwrap();
        assert!(
            list_invitations(
                &pool,
                &crate::test_support::human_principal_for(&pool, *owner).await,
                team_id
            )
            .await
            .unwrap()
            .is_empty(),
            "a revoked invite must not appear in the team listing"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_revoked_is_bad_request(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("rev@e.com")).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "rev@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        revoke_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            inv.id,
        )
        .await
        .unwrap();
        let err = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_creates_membership(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("i@e.com")).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "i@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();

        let resp = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .expect("accept");
        assert_eq!(resp.team_id, team_id);
        assert_eq!(resp.role, TeamRole::Member);
        assert_eq!(resp.team_slug, "acme");

        let role = role_on_team(&pool, team_id, invitee).await.unwrap();
        assert_eq!(role, Some(TeamRole::Member));
    }

    /// C8/P12: the first accept records exactly one `invitation`-via event naming the inviter;
    /// the second accept is a no-op and records none.
    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_is_idempotent(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("i@e.com")).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "i@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .unwrap();
        let events = membership_events(&pool, team_id).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0]["via"], "invitation", "{events:?}");
        assert_eq!(events[0]["profile_id"], serde_json::json!(invitee));
        assert_eq!(events[0]["actor"], serde_json::json!(invitee));
        assert_eq!(events[0]["invited_by"], serde_json::json!(owner));
        let resp = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .expect("idempotent");
        assert_eq!(resp.team_id, team_id);
        assert_eq!(
            membership_events(&pool, team_id).await.len(),
            1,
            "the second accept records nothing"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_unknown_token_not_found(pool: PgPool) {
        let invitee = mk_profile(&pool, "invitee").await;
        let err = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            "deadbeef",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_expired_errors_and_marks_expired(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("i@e.com")).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "i@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        sqlx::query!(
            "UPDATE kb_team_invitations SET expires_at = now() - interval '1 day' WHERE id = $1",
            inv.id
        )
        .execute(&pool)
        .await
        .unwrap();

        let err = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        let status: InvitationStatus = sqlx::query_scalar!(
            r#"SELECT status AS "status: InvitationStatus" FROM kb_team_invitations WHERE id = $1"#,
            inv.id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, InvitationStatus::Expired);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn decline_marks_declined_and_is_idempotent(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("i@e.com")).await;
        let inv = create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "i@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();

        decline_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .expect("decline");
        let status: InvitationStatus = sqlx::query_scalar!(
            r#"SELECT status AS "status: InvitationStatus" FROM kb_team_invitations WHERE id = $1"#,
            inv.id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, InvitationStatus::Declined);
        decline_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .expect("idempotent decline");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_returns_pending_for_manager(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        create_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: "a@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        let list = list_invitations(
            &pool,
            &crate::test_support::human_principal_for(&pool, *owner).await,
            team_id,
        )
        .await
        .expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].invited_email, "a@e.com");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_forbidden_for_non_manager(pool: PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let stranger = mk_profile(&pool, "stranger").await;
        let err = list_invitations(
            &pool,
            &crate::test_support::human_principal_for(&pool, *stranger).await,
            team_id,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    /// **The count and the list must describe the same set.** They are the two readers of
    /// `vw_invitee_invitations`, and the reason the predicate moved into a view is that a second
    /// inline copy would not fail loudly when it drifted — it would report a number that disagreed
    /// with the command it tells the reader to run.
    ///
    /// So this asserts the agreement rather than a literal: the fixture seeds one invitation that
    /// counts and three that must not (declined, expired, and one to a second team that the
    /// caller is also invited to — which does count, so the arithmetic is not trivially 1).
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_count_and_the_list_never_disagree(pool: PgPool) {
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("invitee@x.com")).await;

        let acme = mk_team(&pool, "acme-eng").await;
        add_member(&pool, acme, inviter, "owner").await;
        let other = mk_team(&pool, "other-co").await;
        add_member(&pool, other, inviter, "owner").await;

        seed_invite(&pool, acme, "invitee@x.com", inviter, "pending", 7).await;
        seed_invite(&pool, other, "invitee@x.com", inviter, "pending", 7).await;

        // Neither of these is waiting on anyone. The expired one needs its own team:
        // `idx_invitations_one_pending` is UNIQUE (team_id, invited_email) over live pending
        // rows, so a team cannot hold a live invite and a lapsed one for the same address.
        let stale = mk_team(&pool, "stale-co").await;
        add_member(&pool, stale, inviter, "owner").await;
        seed_invite(&pool, acme, "invitee@x.com", inviter, "declined", 7).await;
        seed_invite(&pool, stale, "invitee@x.com", inviter, "pending", -1).await;

        let listed = list_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
        )
        .await
        .expect("list");
        let counted = count_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            None,
        )
        .await
        .expect("count");

        assert_eq!(listed.len(), 2, "two invitations are actually waiting");
        assert_eq!(
            counted.count,
            listed.len() as i32,
            "the count is the length of the list it stands in for"
        );
        assert_eq!(
            counted.matching, None,
            "no team was asked about, so there is no answer about one"
        );
    }

    /// **`matching` answers about one team, and `None` is not `0`.**
    ///
    /// This is the distinction `temper warmup` spends: `Some(0)` means it asked whether an
    /// invitation grants the context that could not be read and was told no; `None` means it never
    /// asked, because the ref named no team. Only a positive answer entitles the hint to name a
    /// cause, so collapsing the two would let "I did not ask" license the claim.
    #[sqlx::test(migrations = "../../migrations")]
    async fn matching_answers_only_about_the_team_it_was_asked_about(pool: PgPool) {
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("invitee@x.com")).await;

        let acme = mk_team(&pool, "acme-eng").await;
        add_member(&pool, acme, inviter, "owner").await;
        seed_invite(&pool, acme, "invitee@x.com", inviter, "pending", 7).await;

        let held = count_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            Some("acme-eng"),
        )
        .await
        .expect("count");
        assert_eq!(
            held.matching,
            Some(1),
            "the caller does hold one to acme-eng"
        );

        let unrelated = count_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            Some("other-co"),
        )
        .await
        .expect("count");
        assert_eq!(
            unrelated.count, 1,
            "still one invitation waiting on them overall"
        );
        assert_eq!(
            unrelated.matching,
            Some(0),
            "asked about a team they hold nothing for, and told so"
        );

        let unasked = count_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            None,
        )
        .await
        .expect("count");
        assert_eq!(
            unasked.matching, None,
            "not asking is a different answer from being told none"
        );
    }

    /// **One address on two auth links is one waiting invitation, not two.**
    ///
    /// The predicate this view replaced tested `lower(invited_email) IN (subquery)` — set
    /// semantics, which matched a profile once however many links carried the address. A view
    /// joining that subquery directly would match once per link, and a caller who signed in
    /// through two providers with the same email would be told two invitations wait when one
    /// does. `SELECT DISTINCT` in the view is what holds this, and this is its witness.
    ///
    /// Note the second link is a DIFFERENT provider uid with the SAME email: that is the ordinary
    /// shape (one person, two identity providers), not a contrived duplicate.
    #[sqlx::test(migrations = "../../migrations")]
    async fn one_address_on_two_links_counts_once(pool: PgPool) {
        let inviter = mk_profile(&pool, "inviter").await;
        add_auth_email(&pool, inviter, "inviter-uid", Some("owner@x.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-google", Some("invitee@x.com")).await;
        add_auth_email(&pool, invitee, "invitee-github", Some("invitee@x.com")).await;

        let acme = mk_team(&pool, "acme-eng").await;
        add_member(&pool, acme, inviter, "owner").await;
        seed_invite(&pool, acme, "invitee@x.com", inviter, "pending", 7).await;

        let counted = count_for_profile(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            Some("acme-eng"),
        )
        .await
        .expect("count");
        assert_eq!(
            counted.count, 1,
            "one invitation, however many links carry the address"
        );
        assert_eq!(
            counted.matching,
            Some(1),
            "and it is counted once for the team too"
        );
        assert_eq!(
            list_for_profile(
                &pool,
                &crate::test_support::human_principal_for(&pool, *invitee).await
            )
            .await
            .expect("list")
            .len(),
            1,
            "the list agrees, as it must",
        );
    }

    // --- Only the invitee redeems ---

    async fn invite(pool: &PgPool, team_id: Uuid, owner: ProfileId, email: &str) -> TeamInvitation {
        create_invitation(
            pool,
            &crate::test_support::human_principal_for(pool, *owner).await,
            team_id,
            CreateInvitationParams {
                invited_email: email.into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap()
    }

    async fn status_of(pool: &PgPool, id: Uuid) -> InvitationStatus {
        sqlx::query_scalar!(
            r#"SELECT status AS "status: InvitationStatus" FROM kb_team_invitations WHERE id = $1"#,
            id
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// A token in the wrong hands redeems for nobody: a signed-in stranger with their own
    /// verified email is refused, joins nothing, and leaves the invitation pending.
    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_by_someone_other_than_the_invitee_is_refused(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("i@e.com")).await;
        let stranger = mk_profile(&pool, "stranger").await;
        add_auth_email(&pool, stranger, "stranger-uid", Some("stranger@e.com")).await;
        let inv = invite(&pool, team_id, owner, "i@e.com").await;

        let err = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *stranger).await,
            &inv.token,
        )
        .await
        .expect_err("only the invitee may accept");
        assert!(matches!(err, ApiError::ForbiddenDetail(_)), "got {err:?}");
        assert!(
            !err.to_string().contains("i@e.com"),
            "the refusal names no address"
        );
        assert_eq!(role_on_team(&pool, team_id, stranger).await.unwrap(), None);
        assert_eq!(status_of(&pool, inv.id).await, InvitationStatus::Pending);
    }

    /// Holding the invited address on an UNVERIFIED link is not being the invitee.
    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_on_an_unverified_matching_email_is_refused(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let claimant = mk_profile(&pool, "claimant").await;
        add_auth_email_with_verified(&pool, claimant, "claimant-uid", Some("i@e.com"), false).await;
        let inv = invite(&pool, team_id, owner, "i@e.com").await;

        let err = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *claimant).await,
            &inv.token,
        )
        .await
        .expect_err("an unverified address is not the invitee");
        assert!(matches!(err, ApiError::ForbiddenDetail(_)), "got {err:?}");
        assert_eq!(role_on_team(&pool, team_id, claimant).await.unwrap(), None);
    }

    /// An address two profiles have verified addresses nobody (the inbox's rule), so neither
    /// may redeem it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_on_an_address_two_profiles_verify_is_refused(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let one = mk_profile(&pool, "one").await;
        let two = mk_profile(&pool, "two").await;
        add_auth_email(&pool, one, "one-uid", Some("i@e.com")).await;
        add_auth_email(&pool, two, "two-uid", Some("i@e.com")).await;
        let inv = invite(&pool, team_id, owner, "i@e.com").await;

        for who in [one, two] {
            let err = accept_invitation(
                &pool,
                &crate::test_support::human_principal_for(&pool, *who).await,
                &inv.token,
            )
            .await
            .expect_err("an ambiguous address addresses nobody");
            assert!(matches!(err, ApiError::ForbiddenDetail(_)), "got {err:?}");
        }
        assert_eq!(status_of(&pool, inv.id).await, InvitationStatus::Pending);
    }

    /// A stranger cannot spend someone else's invitation by declining it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn decline_by_someone_other_than_the_invitee_is_refused(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("i@e.com")).await;
        let stranger = mk_profile(&pool, "stranger").await;
        add_auth_email(&pool, stranger, "stranger-uid", Some("stranger@e.com")).await;
        let inv = invite(&pool, team_id, owner, "i@e.com").await;

        let err = decline_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *stranger).await,
            &inv.token,
        )
        .await
        .expect_err("only the invitee may decline");
        assert!(matches!(err, ApiError::ForbiddenDetail(_)), "got {err:?}");
        assert_eq!(status_of(&pool, inv.id).await, InvitationStatus::Pending);
    }

    /// The invitee still redeems after a refused attempt, and the address matches case-blind,
    /// as the inbox does.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_invitee_accepts_after_a_strangers_refused_attempt(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let stranger = mk_profile(&pool, "stranger").await;
        add_auth_email(&pool, stranger, "stranger-uid", Some("stranger@e.com")).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("Invitee@E.com")).await;
        let inv = invite(&pool, team_id, owner, "invitee@e.com").await;

        accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *stranger).await,
            &inv.token,
        )
        .await
        .expect_err("refused");
        let resp = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .expect("the invitee accepts");
        assert_eq!(resp.team_id, team_id);
        assert_eq!(
            role_on_team(&pool, team_id, invitee).await.unwrap(),
            Some(TeamRole::Member)
        );
    }

    /// Declining an expired invitation changes nothing: it is already spent. The first call meets
    /// a pending row past its expiry (flipped to `expired`), the second an `expired` row.
    #[sqlx::test(migrations = "../../migrations")]
    async fn declining_an_expired_invitation_leaves_it_expired(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("lapsed@e.com")).await;
        seed_invite(&pool, team_id, "lapsed@e.com", owner, "pending", -1).await;
        let token: String = sqlx::query_scalar(
            "SELECT token FROM kb_team_invitations WHERE invited_email = 'lapsed@e.com'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        for _ in 0..2 {
            decline_invitation(
                &pool,
                &crate::test_support::human_principal_for(&pool, *invitee).await,
                &token,
            )
            .await
            .expect("declining a spent invitation is a no-op");
        }
        let status: String =
            sqlx::query_scalar("SELECT status::text FROM kb_team_invitations WHERE token = $1")
                .bind(&token)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "expired");
    }

    /// The real invitee to a team that has since been deleted is told so, not told the
    /// invitation belongs to someone else, and joins nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn accepting_for_a_deleted_team_says_so(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        add_auth_email(&pool, invitee, "invitee-uid", Some("i@e.com")).await;
        let inv = invite(&pool, team_id, owner, "i@e.com").await;
        sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
            .bind(team_id)
            .execute(&pool)
            .await
            .unwrap();

        let err = accept_invitation(
            &pool,
            &crate::test_support::human_principal_for(&pool, *invitee).await,
            &inv.token,
        )
        .await
        .expect_err("a deleted team admits nobody");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.contains("no longer exists")),
            "got {err:?}"
        );
        assert_eq!(role_on_team(&pool, team_id, invitee).await.unwrap(), None);
    }
}
