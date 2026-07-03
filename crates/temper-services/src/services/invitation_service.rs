//! Team invitation service over `kb_team_invitations`.
//!
//! Service-direct: no Backend-trait command, no event emission — invitations are
//! provisioning/infra, the same precedent as `team_service` / `context_service`.
//! Authorization precedes every write, reusing `team_service::role_on_team` +
//! `can_manage`. Tokens are 128-bit CSPRNG values, never UUIDs (which are
//! time-sortable and guessable).

use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::team_service::{can_manage, role_on_team};
use temper_core::types::ids::ProfileId;
use temper_core::types::invitation::{AcceptInvitationResponse, InvitationStatus, TeamInvitation};
use temper_core::types::team::TeamRole;

/// Parameters for creating an invitation.
pub struct CreateInvitationParams {
    pub invited_email: String,
    pub role: TeamRole,
}

/// Mint a 128-bit capability token, hex-encoded (32 chars). CSPRNG-backed —
/// NOT a UUID (which is time-sortable and guessable).
fn mint_token() -> String {
    let mut rng = rand::rngs::OsRng;
    format!("{:016x}{:016x}", rng.next_u64(), rng.next_u64())
}

/// Create a pending invitation. Auth: caller must own/maintain the team.
/// `Owner` role is rejected. A second pending invite for the same
/// `(team, email)` conflicts (partial unique index `idx_invitations_one_pending`).
pub async fn create_invitation(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    params: CreateInvitationParams,
) -> ApiResult<TeamInvitation> {
    // Auth before writes.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }
    if params.role == TeamRole::Owner {
        return Err(ApiError::BadRequest(
            "ownership is transferred, not invited".to_string(),
        ));
    }

    let id = Uuid::now_v7();
    let token = mint_token();
    let row = sqlx::query_as!(
        TeamInvitation,
        r#"
        INSERT INTO kb_team_invitations
            (id, team_id, invited_email, invited_by_profile_id, role, token, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending')
        RETURNING id, team_id, invited_email, invited_by_profile_id,
                  role AS "role: TeamRole", token,
                  status AS "status: InvitationStatus", expires_at, created
        "#,
        id,
        team_id,
        params.invited_email,
        *caller,
        params.role as TeamRole,
        token,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            ApiError::Conflict("a pending invitation already exists for this email".to_string())
        }
        _ => ApiError::from(e),
    })?;

    Ok(row)
}

/// Redeem an invitation token (bearer authority — the token IS the authority;
/// membership is created for `caller`). Idempotent. Expiry is checked lazily
/// here and flips the row to `expired`.
pub async fn accept_invitation(
    pool: &PgPool,
    caller: ProfileId,
    token: &str,
) -> ApiResult<AcceptInvitationResponse> {
    let inv = sqlx::query_as!(
        TeamInvitation,
        r#"
        SELECT id, team_id, invited_email, invited_by_profile_id,
               role AS "role: TeamRole", token,
               status AS "status: InvitationStatus", expires_at, created
          FROM kb_team_invitations
         WHERE token = $1
        "#,
        token,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    let team_slug = sqlx::query_scalar!("SELECT slug FROM kb_teams WHERE id = $1", inv.team_id)
        .fetch_one(pool)
        .await?;

    match inv.status {
        InvitationStatus::Accepted => {
            // Idempotent iff caller is already the member.
            match role_on_team(pool, inv.team_id, caller).await? {
                Some(role) => Ok(AcceptInvitationResponse {
                    team_id: inv.team_id,
                    team_slug,
                    role,
                }),
                None => Err(ApiError::Conflict(
                    "invitation already redeemed".to_string(),
                )),
            }
        }
        InvitationStatus::Declined => {
            Err(ApiError::BadRequest("invitation was declined".to_string()))
        }
        InvitationStatus::Expired => {
            Err(ApiError::BadRequest("invitation has expired".to_string()))
        }
        InvitationStatus::Pending => {
            if inv.expires_at < chrono::Utc::now() {
                sqlx::query!(
                    "UPDATE kb_team_invitations SET status = 'expired' WHERE id = $1",
                    inv.id,
                )
                .execute(pool)
                .await?;
                return Err(ApiError::BadRequest("invitation has expired".to_string()));
            }

            let mut tx = pool.begin().await?;
            sqlx::query!(
                r#"
                INSERT INTO kb_team_members (team_id, profile_id, role)
                VALUES ($1, $2, $3)
                ON CONFLICT (team_id, profile_id) DO NOTHING
                "#,
                inv.team_id,
                *caller,
                inv.role as TeamRole,
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!(
                "UPDATE kb_team_invitations SET status = 'accepted' WHERE id = $1",
                inv.id,
            )
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(AcceptInvitationResponse {
                team_id: inv.team_id,
                team_slug,
                role: inv.role,
            })
        }
    }
}

/// Decline an invitation (bearer authority). Idempotent if already declined;
/// declining an accepted invitation is a `BadRequest`.
pub async fn decline_invitation(pool: &PgPool, _caller: ProfileId, token: &str) -> ApiResult<()> {
    let status = sqlx::query_scalar!(
        r#"SELECT status AS "status: InvitationStatus" FROM kb_team_invitations WHERE token = $1"#,
        token,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    match status {
        InvitationStatus::Declined => Ok(()),
        InvitationStatus::Accepted => Err(ApiError::BadRequest(
            "invitation was already accepted".to_string(),
        )),
        InvitationStatus::Pending | InvitationStatus::Expired => {
            sqlx::query!(
                "UPDATE kb_team_invitations SET status = 'declined' WHERE token = $1",
                token,
            )
            .execute(pool)
            .await?;
            Ok(())
        }
    }
}

/// List pending, non-expired invitations for a team. Auth: owner/maintainer.
pub async fn list_invitations(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
) -> ApiResult<Vec<TeamInvitation>> {
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }
    let rows = sqlx::query_as!(
        TeamInvitation,
        r#"
        SELECT id, team_id, invited_email, invited_by_profile_id,
               role AS "role: TeamRole", token,
               status AS "status: InvitationStatus", expires_at, created
          FROM kb_team_invitations
         WHERE team_id = $1 AND status = 'pending' AND expires_at > now()
         ORDER BY created DESC
        "#,
        team_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::types::team::TeamRole;

    /// Insert a profile with the given handle, return its ProfileId.
    async fn mk_profile(pool: &PgPool, handle: &str) -> ProfileId {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_by_owner_succeeds(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let inv = create_invitation(
            &pool,
            owner,
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_rejects_owner_role(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let err = create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams {
                invited_email: "a@e.com".into(),
                role: TeamRole::Owner,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_non_manager_forbidden(pool: PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let stranger = mk_profile(&pool, "stranger").await;
        let err = create_invitation(
            &pool,
            stranger,
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
        create_invitation(&pool, owner, team_id, p()).await.unwrap();
        let err = create_invitation(&pool, owner, team_id, p())
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn fresh_invite_succeeds_after_decline(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        let inv = create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams {
                invited_email: "again@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        decline_invitation(&pool, invitee, &inv.token)
            .await
            .unwrap();
        // A new pending invite for the same (team, email) is now allowed.
        create_invitation(
            &pool,
            owner,
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
    async fn accept_creates_membership(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        let inv = create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams {
                invited_email: "i@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();

        let resp = accept_invitation(&pool, invitee, &inv.token)
            .await
            .expect("accept");
        assert_eq!(resp.team_id, team_id);
        assert_eq!(resp.role, TeamRole::Member);
        assert_eq!(resp.team_slug, "acme");

        let role = role_on_team(&pool, team_id, invitee).await.unwrap();
        assert_eq!(role, Some(TeamRole::Member));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_is_idempotent(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        let inv = create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams {
                invited_email: "i@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        accept_invitation(&pool, invitee, &inv.token).await.unwrap();
        let resp = accept_invitation(&pool, invitee, &inv.token)
            .await
            .expect("idempotent");
        assert_eq!(resp.team_id, team_id);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_unknown_token_not_found(pool: PgPool) {
        let invitee = mk_profile(&pool, "invitee").await;
        let err = accept_invitation(&pool, invitee, "deadbeef")
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::NotFound));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_expired_errors_and_marks_expired(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let invitee = mk_profile(&pool, "invitee").await;
        let inv = create_invitation(
            &pool,
            owner,
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

        let err = accept_invitation(&pool, invitee, &inv.token)
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
        let inv = create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams {
                invited_email: "i@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();

        decline_invitation(&pool, invitee, &inv.token)
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
        decline_invitation(&pool, invitee, &inv.token)
            .await
            .expect("idempotent decline");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_returns_pending_for_manager(pool: PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams {
                invited_email: "a@e.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .unwrap();
        let list = list_invitations(&pool, owner, team_id).await.expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].invited_email, "a@e.com");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_forbidden_for_non_manager(pool: PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let stranger = mk_profile(&pool, "stranger").await;
        let err = list_invitations(&pool, stranger, team_id)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }
}
