//! The seam between the pure admission machines and the database (spec 2026-07-20 §4).
//!
//! ```text
//! services gathers evidence ─► temper-principal decides ─► ONE SQL function commits
//!                                                          (row + log + event, one txn)
//! ```
//!
//! `temper-principal` never resolves a credential and holds no identifiers — it judges assembled
//! evidence, which is what makes it safe to share across surfaces. Every id in this file stays on
//! this side of the boundary.

use crate::error::{ApiError, ApiResult};
use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_principal::{
    admit as pure_admit, transition, Act, ActorAuthority, AdmittedPrincipal, Provisioner, Refusal,
    Standing,
};

/// Parameters for one standing transition. A params struct because the domain arguments would
/// otherwise exceed the repo's threshold, and because `authority` and `actor` must travel together.
pub struct ApplyStandingParams {
    /// The principal whose standing changes.
    pub subject: ProfileId,
    pub act: Act,
    /// The acting principal. `None` for credential-authority acts and the boot-seed.
    pub actor: Option<ProfileId>,
    pub authority: ActorAuthority,
}

/// Load a principal's current standing. `Ok(None)` means no row — which denies (spec §7).
pub async fn load(pool: &PgPool, profile_id: ProfileId) -> ApiResult<Option<Standing>> {
    let raw: Option<String> = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
        *profile_id
    )
    .fetch_optional(pool)
    .await?;

    // A row whose value this binary does not recognize is NOT `None` — that would silently
    // downgrade "unknown state" to "no standing" and lose the distinction the refusal needs.
    match raw {
        None => Ok(None),
        Some(r) => Standing::parse(&r).map(Some).ok_or_else(|| {
            ApiError::Internal(format!(
                "unrecognized standing {r:?} for profile {}",
                *profile_id
            ))
        }),
    }
}

/// The per-request admission decision (Level 2).
///
/// Reads standing and nothing else (D15 obligation 1). A `Revoked` principal is refused whether or
/// not a review is pending; ANDing the marker in would restore the conjunction-across-provisional-
/// facts shape D2 forbids, and it is the tempting change.
pub async fn admit(pool: &PgPool, profile_id: ProfileId) -> Result<AdmittedPrincipal, Refusal> {
    let raw: Option<String> = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
        *profile_id
    )
    .fetch_optional(pool)
    .await
    .map_err(|_| Refusal::NoStanding)?;

    pure_admit(raw.as_deref())
}

/// Decide, then commit. **The order is not negotiable** — auth before writes, and it is also what
/// keeps the SQL committer free of a second transition table.
pub async fn apply(pool: &PgPool, params: ApplyStandingParams) -> ApiResult<Standing> {
    let current = load(pool, params.subject).await?;

    // `Reactivate` is THE ONLY data-dependent target in the machine (spec §6), so it is the only
    // act that needs a read before the decision. Treat a second such act as a design smell until
    // argued for.
    let act = match params.act {
        Act::Reactivate { prior: None } => {
            let prior: Option<String> =
                sqlx::query_scalar!("SELECT principal_prior_standing($1)", *params.subject)
                    .fetch_one(pool)
                    .await?;
            Act::Reactivate {
                prior: prior.as_deref().and_then(Standing::parse),
            }
        }
        other => other,
    };

    // Decide. A refusal carries a human reason. INTERIM mapping (Beat H / Task 17 replaces this
    // whole thing with the typed `Refusal` carried on `ApiError::SystemAccessRequired`): a refused
    // transition is a 4xx that names why. `Forbidden` (payload-less) would drop the reason the
    // caller and the test both need, so the reason rides `BadRequest`/`Conflict`. The one contract
    // we must preserve NOW is the 409 the DB unique index used to give a duplicate join request
    // (D12 makes `requested` standing the duplicate guard, so the index no longer fires) — the
    // deployed CLI's "you already have a pending request" branch keys on it.
    let resulting = transition(current, &act, params.authority).map_err(refusal_to_api_error)?;

    let reason = match &act {
        Act::Revoke { reason } => Some(reason.clone()),
        _ => None,
    };

    let committed: Option<String> = sqlx::query_scalar!(
        "SELECT principal_standing_apply($1,$2,$3,$4,$5)",
        *params.subject,
        act_name(&act),
        resulting.as_str(),
        params.actor.map(|a| *a),
        reason,
    )
    .fetch_one(pool)
    .await?;

    // The committer echoes back what it wrote. A disagreement means the SQL grew an opinion.
    debug_assert_eq!(committed.as_deref(), Some(resulting.as_str()));
    Ok(resulting)
}

/// The database literal for an act. Exhaustive, no catchall — adding an act is a compile error.
fn act_name(act: &Act) -> &'static str {
    match act {
        Act::Provision { .. } => "provision",
        Act::Request => "request",
        Act::Withdraw => "withdraw",
        Act::Approve => "approve",
        Act::Reject => "reject",
        Act::Revoke { .. } => "revoke",
        Act::Deactivate => "deactivate",
        Act::Reactivate { .. } => "reactivate",
        Act::RequestReview => "request_review",
    }
}

/// Map a machine refusal to the interim HTTP-shaped error. An "already in a non-terminal state"
/// refusal — re-`Request`ing while `Requested`, or acting on an already-`Approved` principal — is a
/// conflict with current state, not a malformed request, so it keeps the 409 the DB unique index
/// used to give a duplicate join request (the deployed CLI's Conflict branch keys on it). Everything
/// else is the interim `BadRequest` (a 4xx that names why). Task 17 supersedes this whole mapping
/// with the typed `Refusal` carried on `ApiError::SystemAccessRequired`.
fn refusal_to_api_error(refusal: Refusal) -> ApiError {
    match &refusal {
        Refusal::Requested
        | Refusal::IllegalTransition {
            from: Some(Standing::Requested) | Some(Standing::Approved),
            ..
        } => ApiError::Conflict(refusal.reason()),
        _ => ApiError::BadRequest(refusal.reason()),
    }
}

/// Convenience for the four mint doors (D11): every one births `Denied`, except genesis.
pub async fn provision(
    pool: &PgPool,
    subject: ProfileId,
    path: Provisioner,
) -> ApiResult<Standing> {
    apply(
        pool,
        ApplyStandingParams {
            subject,
            act: Act::Provision { path },
            actor: None,
            authority: ActorAuthority::Credential,
        },
    )
    .await
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn profile(pool: &PgPool, handle: &str) -> ProfileId {
        let id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap();
        ProfileId::from(id)
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_illegal_transition_is_refused_and_writes_nothing(pool: PgPool) {
        let p = profile(&pool, "illegal").await;
        let admin = profile(&pool, "illegal-admin").await;
        apply(
            &pool,
            ApplyStandingParams {
                subject: p,
                act: Act::Provision {
                    path: Provisioner::OauthFirstLogin,
                },
                actor: None,
                authority: ActorAuthority::Credential,
            },
        )
        .await
        .unwrap();

        // Revoke from Denied — you cannot revoke what was never granted (spec §6).
        let err = apply(
            &pool,
            ApplyStandingParams {
                subject: p,
                act: Act::Revoke {
                    reason: "no".into(),
                },
                actor: Some(admin),
                authority: ActorAuthority::Admin,
            },
        )
        .await
        .expect_err("must refuse");

        assert!(
            format!("{err}").contains("not legal"),
            "the refusal must carry a reason: {err}"
        );

        let state: String =
            sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
                .bind(*p)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "denied", "a refused act must write nothing");

        let logs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_principal_standing_events WHERE profile_id=$1 AND act='revoke'",
        )
        .bind(*p)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(logs, 0, "a refused act must not appear in the log");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reactivate_restores_the_prior_state_through_the_seam(pool: PgPool) {
        let p = profile(&pool, "react").await;
        let admin = profile(&pool, "react-admin").await;
        for (act, auth) in [
            (
                Act::Provision {
                    path: Provisioner::OauthFirstLogin,
                },
                ActorAuthority::Credential,
            ),
            (Act::Approve, ActorAuthority::Admin),
            (Act::Deactivate, ActorAuthority::Admin),
        ] {
            apply(
                &pool,
                ApplyStandingParams {
                    subject: p,
                    act,
                    actor: Some(admin),
                    authority: auth,
                },
            )
            .await
            .unwrap();
        }

        let restored = apply(
            &pool,
            ApplyStandingParams {
                subject: p,
                act: Act::Reactivate { prior: None }, // the seam fills this in
                actor: Some(admin),
                authority: ActorAuthority::Admin,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            restored,
            Standing::Approved,
            "Reactivate restores rather than guesses (§5)"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn admit_denies_a_principal_with_no_standing_row(pool: PgPool) {
        let p = profile(&pool, "nostanding").await;
        assert_eq!(admit(&pool, p).await, Err(Refusal::NoStanding));
    }
}
