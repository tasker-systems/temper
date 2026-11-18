//! Team invitation service over `kb_team_invitations`.
//!
//! Service-direct: no `DbBackend` command, no event emission — invitations are
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
/// to `maintainer`), and a system admin any role; on the gating team and temper-system only a
/// system admin invites (R11). The offered role is not checked against any membership row: an invitation is
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
        MembershipAuthority::resolve_precheck(&mut conn, caller, team_id)
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
        if !MembershipAuthority::resolve_precheck(&mut conn, caller, team_id)
            .await?
            .manages()
        {
            return Err(ApiError::Forbidden);
        }
    }

    // Under the team lock (taken before any invitation row, the order `membership_write` holds),
    // the authority is decided again: a demotion or role change committed since the pool check
    // refuses the revoke, and a demotion in flight waits for it.
    let mut tx = pool.begin().await?;
    team_membership::lock_team(&mut tx, team_id).await?;
    if !MembershipAuthority::resolve(&mut tx, caller, team_id)
        .await?
        .manages()
    {
        return Err(ApiError::Forbidden);
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
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;

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
        if !MembershipAuthority::resolve_precheck(&mut conn, caller, team_id)
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
mod tests;
