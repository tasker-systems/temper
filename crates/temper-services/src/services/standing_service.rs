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
use temper_core::types::access_gate::JoinRequestStatus;
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
    let mut conn = pool.acquire().await?;
    load_conn(&mut conn, profile_id).await
}

/// Connection-taking twin of [`load`], so a caller already inside a transaction reads the
/// standing its own uncommitted writes would have produced rather than the pre-transaction row.
pub(crate) async fn load_conn(
    conn: &mut sqlx::PgConnection,
    profile_id: ProfileId,
) -> ApiResult<Option<Standing>> {
    let raw: Option<String> = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
        *profile_id
    )
    .fetch_optional(&mut *conn)
    .await?;

    interpret_standing(raw, profile_id)
}

/// Connection-taking twin of [`load_conn`] that takes a **row lock** (`FOR UPDATE`) on the
/// standing row. [`apply`] uses this so that two concurrent transitions on the same subject
/// serialize behind the lock instead of both reading the same `current`, both judging a legal
/// transition against it, and the second clobbering the first — the check-then-act gap the whole
/// admission design exists to remove. When there is no row yet the lock takes nothing; the only
/// act legal from absence is `Provision`, whose committer upserts with `ON CONFLICT`, so a racing
/// pair still converges rather than corrupting.
async fn load_locked(
    conn: &mut sqlx::PgConnection,
    profile_id: ProfileId,
) -> ApiResult<Option<Standing>> {
    let raw: Option<String> = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1 FOR UPDATE",
        *profile_id
    )
    .fetch_optional(&mut *conn)
    .await?;

    interpret_standing(raw, profile_id)
}

/// Turn a raw `state` column into a `Standing`. A value this binary does not recognize is NOT
/// `None` — that would silently downgrade "unknown state" to "no standing" and lose the
/// distinction the refusal needs.
fn interpret_standing(raw: Option<String>, profile_id: ProfileId) -> ApiResult<Option<Standing>> {
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
///
/// The read, the decision, the commit, and the governance demotion all run in **one transaction**,
/// and the read takes a `FOR UPDATE` row lock. This closes two check-then-act gaps that the
/// per-connection version carried:
///   1. Two concurrent transitions on one subject could each read the same `current`, each judge a
///      legal transition, and the second silently clobber the first. The row lock serializes them,
///      so the loser re-reads the winner's committed state and its transition is re-judged against
///      reality.
///   2. The "admin implies approved" invariant (§9) was a *second* statement after the standing
///      write on the pool; if the standing `Revoke` committed but the governance demotion then
///      failed, a principal could be left `revoked` yet still holding governance. Sharing one transaction
///      makes the pair atomic — both land or neither. (`is_system_admin` also requires approved
///      standing since 20261205000010, so such a row would confer nothing; the demotion still
///      ends it.)
pub async fn apply(pool: &PgPool, params: ApplyStandingParams) -> ApiResult<Standing> {
    let mut tx = pool.begin().await?;
    let resulting = apply_conn(&mut tx, params).await?;
    tx.commit().await?;
    Ok(resulting)
}

/// Connection-taking twin of [`apply`], for a caller whose standing write must be atomic with
/// writes of its own — a join-request decision, an admin promotion. Same lock, same decision, same
/// commit and demotion; the caller owns the transaction and its commit. A refusal is mapped with
/// the interim [`refusal_to_api_error`]; a caller that must word the refusal itself uses
/// [`judge_and_apply_conn`].
pub(crate) async fn apply_conn(
    conn: &mut sqlx::PgConnection,
    params: ApplyStandingParams,
) -> ApiResult<Standing> {
    match judge_and_apply_conn(conn, params, None).await? {
        Judged::Moved(resulting) => Ok(resulting),
        Judged::Refused { refusal, .. } => Err(refusal_to_api_error(refusal)),
    }
}

/// What [`judge_and_apply_conn`] decided. `Refused` carries the standing the refusal was judged
/// against, so a caller can tell the person what they hit ("this person is now Revoked") rather
/// than only which act was illegal.
pub(crate) enum Judged {
    Moved(Standing),
    Refused {
        current: Option<Standing>,
        refusal: Refusal,
    },
}

/// Lock the subject's standing row in the caller's transaction, ahead of any other row the caller
/// is about to take. Standing is the first lock every standing act takes ([`apply`] reads it
/// `FOR UPDATE` before anything else), so a caller that locks other rows before reaching
/// [`apply_conn`] must take this one first or invert the order against a concurrent `apply` on
/// the same subject.
pub(crate) async fn lock_conn(
    conn: &mut sqlx::PgConnection,
    subject: ProfileId,
) -> ApiResult<Option<Standing>> {
    load_locked(conn, subject).await
}

/// The body of [`apply`] on the caller's connection: lock, judge, and — only when the transition
/// is legal — commit. **A refusal writes nothing**, so the caller may return an error and let its
/// transaction roll back with no standing row, log entry or event behind it.
///
/// `note` is free text recorded as the log entry's reason for an act that carries none of its own
/// — a join decision's note, "system admin promotion". `Revoke`'s reason is the act's, and wins.
pub(crate) async fn judge_and_apply_conn(
    conn: &mut sqlx::PgConnection,
    params: ApplyStandingParams,
    note: Option<String>,
) -> ApiResult<Judged> {
    let tx = conn;

    // Lock the subject's standing row for the life of the transaction, then read it. A concurrent
    // `apply` on the same subject blocks here until we commit.
    let current = load_locked(&mut *tx, params.subject).await?;
    // (The loader takes a bare connection so both the pooled and transactional callers share one
    // body; a caller that already took this lock through [`lock_conn`] simply holds it already.)

    // `Reactivate` is THE ONLY data-dependent target in the machine (spec §6), so it is the only
    // act that needs a read before the decision. Treat a second such act as a design smell until
    // argued for. Read it on the same connection so it sees our locked, consistent view.
    let act = match params.act {
        Act::Reactivate { prior: None } => {
            let prior: Option<String> =
                sqlx::query_scalar!("SELECT principal_prior_standing($1)", *params.subject)
                    .fetch_one(&mut *tx)
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
    // deployed CLI's "you already have a pending request" branch keys on it. A refusal returns
    // before the first write, so nothing is written; the caller's transaction rolls back clean.
    let resulting = match transition(current, &act, params.authority) {
        Ok(resulting) => resulting,
        Err(refusal) => return Ok(Judged::Refused { current, refusal }),
    };

    let reason = match &act {
        Act::Revoke { reason } => Some(reason.clone()),
        _ => note,
    };

    let committed: Option<String> = sqlx::query_scalar!(
        "SELECT principal_standing_apply($1,$2,$3,$4,$5)",
        *params.subject,
        act_name(&act),
        resulting.as_str(),
        params.actor.map(|a| *a),
        reason,
    )
    .fetch_one(&mut *tx)
    .await?;

    // The committer echoes back what it wrote. A disagreement means the SQL grew an opinion.
    debug_assert_eq!(committed.as_deref(), Some(resulting.as_str()));

    // §9 — Revoke and Deactivate demote, so "admin, but admission revoked" is never representable.
    // The invariant is maintained BY TRANSITION: a one-directional write (admission → governance).
    // Since 20261205000010 `is_system_admin` also ANDs live standing at read, as defence in depth,
    // so a row a missed demotion leaves behind confers nothing. Fired unconditionally on
    // these two terminals — `principal_governance_set(_, false, _)` is a no-op on the common case of
    // a principal that was never an admin. Task 14 routes machine-credential revocation through this
    // same `apply`, so the hook also fires when a machine is credential-revoked from `Approved`: a
    // harmless no-op there too. In the SAME transaction as the standing write, so the two are atomic.
    if matches!(resulting, Standing::Revoked | Standing::Deactivated) {
        sqlx::query_scalar!(
            "SELECT principal_governance_set($1, false, $2, $3)",
            *params.subject,
            params.actor.map(|a| *a),
            Some(format!("demoted by {}", act_name(&act))),
        )
        .fetch_one(&mut *tx)
        .await?;

        // The same two terminals end the principal's live refresh chains, in the same transaction
        // and for the same reason the demotion is here: ending a standing should end the
        // credentials that standing backed in the same commit, rather than leaving the API gate as
        // the only thing between a held credential and a refusal.
        //
        // "In the same commit" is the honest claim, and it is narrower than "atomically with
        // respect to everything". The AS rotates over an HTTP driver with no interactive
        // transaction, so a rotation already past its own admission check can still insert its
        // successor after this commit lands — leaving one live row for a principal this just
        // revoked. That row is refused at ITS next rotation, so the excursion is one token pair
        // wide; it is named because an operator auditing "live chains for revoked principals"
        // straight after a revoke can legitimately see one. Defence in depth for the
        // gate, and it generalizes what `slack_disconnect_service::revoke_as_refresh_token` had
        // done for its own single token.
        //
        // Keyed on `profile_id`, which the AS stamps at issue time through
        // `/internal/principal/resolve`. A row whose owner could not be resolved (fail-open login)
        // is NOT reached here and is held by `chain_expires_at` alone until its next rotation
        // re-resolves it — stated in 20260825000010's header, not silently absorbed.
        //
        // Machines never match: `client_credentials` issues no refresh token (endpoints.ts's
        // `MachineTokenResponse` carries none), so this is a no-op on that arm rather than a
        // behaviour it quietly acquires.
        let chains_ended = sqlx::query!(
            r#"
            UPDATE kb_oauth_refresh_tokens
               SET revoked_at = now()
             WHERE profile_id = $1
               AND revoked_at IS NULL
            "#,
            *params.subject,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();

        // Say how many. Without this the call is indistinguishable, to the operator and to the
        // ledger, between "ended three live sessions" and "matched nothing" — and matching nothing
        // is the expected outcome of every misconfiguration on the AS side, where the owner never
        // got recorded. A revoke that quietly ends no credentials is the failure this hook exists
        // to prevent, so it must not report success in the same words as one that works.
        //
        // Emitted HERE, in Rust, deliberately: this surface is instrumented and ships to Tempo,
        // and the AS surface that would otherwise have to report it is not.
        //
        // Zero is not by itself an error — a principal may genuinely hold no live chain — so it is
        // a warning to correlate, not a failure to act on blindly.
        if chains_ended == 0 {
            // Zero on its own is a poor signal and would be a poor detector: it is the guaranteed
            // outcome for every machine principal (`client_credentials` mints no refresh token) and
            // for any human who simply was not signed in. So carry the fact that actually separates
            // "nothing to end" from "the AS is not recording owners" — the number of LIVE chains on
            // the instance belonging to nobody. That is a property of the deployment rather than of
            // this subject, it is what a missing `INTERNAL_RESOLVE_URL` produces, and it is
            // index-covered by `idx_kb_oauth_refresh_tokens_live_profile`.
            //
            // Read only on this branch: the common, healthy path pays nothing for it.
            let ownerless: i64 = sqlx::query_scalar!(
                r#"
                SELECT count(*) AS "count!"
                  FROM kb_oauth_refresh_tokens
                 WHERE profile_id IS NULL
                   AND revoked_at IS NULL
                "#,
            )
            .fetch_one(&mut *tx)
            .await?;

            tracing::warn!(
                subject = %params.subject,
                act = act_name(&act),
                ownerless_live_chains = ownerless,
                "standing terminal ended no refresh chains — unremarkable if the principal held \
                 none, but a non-zero ownerless_live_chains means the authorization server is not \
                 recording chain owners and no revoke can end a session"
            );
        } else {
            tracing::info!(
                subject = %params.subject,
                act = act_name(&act),
                chains_ended,
                "standing terminal ended live refresh chains"
            );
        }
    }

    // An admission decision taken from `Requested` answers the person's open join request, in
    // this same transaction, so a pending row can never outlive the standing it was filed under.
    // Without this a direct `admin approve` (or a promotion) left the request pending; a later
    // revoke then made it a stale row whose rejection would have laundered Revoked into Denied.
    //
    // Approve and Reject only — the admin's admission decisions. `Withdraw` is the person's own
    // act and its door closes its own row. `Deactivate` deliberately does NOT close it:
    // `Reactivate` restores `Requested`, and a request closed on the way down would come back as
    // a `Requested` person with nothing in the admin queue. A deactivated person's pending request
    // is instead parked — deciding it is refused until the person is reactivated.
    //
    // Only the GATING team's request: that is the one an admission decision answers. A pending
    // row on some other team (a gating team since changed) is left for its own decision rather
    // than marked approved without the membership an approval of it would write.
    //
    // `review_request` closes its own row before it reaches here; for that door this matches
    // nothing unless the person also holds a second pending gating-team row, which the
    // per-(team, profile) unique index forbids.
    if current == Some(Standing::Requested) {
        let closed_as = match &act {
            Act::Approve => Some(JoinRequestStatus::Approved),
            Act::Reject => Some(JoinRequestStatus::Rejected),
            Act::Provision { .. }
            | Act::Request
            | Act::Withdraw
            | Act::Revoke { .. }
            | Act::Deactivate
            | Act::Reactivate { .. }
            | Act::RequestReview => None,
        };
        if let Some(status) = closed_as {
            sqlx::query!(
                r#"
                UPDATE kb_join_requests
                   SET status = $2,
                       reviewed_by_profile_id = $3,
                       reviewed_at = now(),
                       decision_note = $4,
                       updated = now()
                 WHERE requesting_profile_id = $1
                   AND status = 'pending'
                   AND team_id = (SELECT t.id
                                    FROM kb_teams t
                                    JOIN kb_system_settings s ON s.gating_team_slug = t.slug
                                   WHERE s.id = 1)
                "#,
                *params.subject,
                status as JoinRequestStatus,
                params.actor.map(|a| *a),
                JOIN_REQUEST_CLOSED_BY_STANDING_DECISION,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    Ok(Judged::Moved(resulting))
}

/// Recorded on a join request that an admission decision answered outside the request itself — a
/// direct `admin approve`, or a promotion to system admin.
pub const JOIN_REQUEST_CLOSED_BY_STANDING_DECISION: &str =
    "closed by a direct admission decision — the person's standing was decided outside this request";

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

/// Connection-taking twin of [`provision`], so a mint door can birth standing **inside the same
/// transaction that creates the profile it belongs to**.
///
/// [`provision`] takes `&PgPool` and therefore writes on a *different* connection. Called from
/// inside a transaction it commits independently of that transaction, so a rollback leaves the
/// standing row behind — and, in the failure that matters, lets the profile commit while the work
/// that was supposed to accompany it does not. `machine_registration_service::provision` hit this
/// first and worked around it with a raw `principal_standing_apply` call carrying a hardcoded
/// `'denied'` (see its comment at the D11 write); this keeps the decision in [`transition`], where
/// changing what a door births stays a one-place change.
///
/// Deliberately narrower than [`apply`]: `Provision` is the only act it serves, and that act needs
/// neither the `Reactivate` pre-read (the machine's only data-dependent target) nor the
/// `Revoke`/`Deactivate` governance demotion. Both are unreachable here, so carrying them would be
/// dead code implying a coupling that does not exist.
pub(crate) async fn provision_conn(
    conn: &mut sqlx::PgConnection,
    subject: ProfileId,
    path: Provisioner,
) -> ApiResult<Standing> {
    let current = load_conn(&mut *conn, subject).await?;

    // Decide, then commit — the same non-negotiable order [`apply`] states.
    let act = Act::Provision { path };
    let resulting =
        transition(current, &act, ActorAuthority::Credential).map_err(refusal_to_api_error)?;

    let committed: Option<String> = sqlx::query_scalar!(
        "SELECT principal_standing_apply($1,$2,$3,$4,$5)",
        *subject,
        act_name(&act),
        resulting.as_str(),
        None::<uuid::Uuid>,
        None::<String>,
    )
    .fetch_one(&mut *conn)
    .await?;

    // The committer echoes back what it wrote. A disagreement means the SQL grew an opinion.
    debug_assert_eq!(committed.as_deref(), Some(resulting.as_str()));

    Ok(resulting)
}

/// Convenience for the four mint doors (D11): every one births `Denied`, except genesis.
///
/// Delegates to the crate-private `provision_conn` rather than calling [`apply`] directly, so the
/// two provision paths cannot drift. They were briefly identical-by-coincidence — `apply`'s
/// `Reactivate` pre-read and its `Revoke`/`Deactivate` demotion are both unreachable for
/// `Provision`, and its `actor` and `reason` are both `None` here — and "identical by coincidence"
/// is what a shared implementation is for.
pub async fn provision(
    pool: &PgPool,
    subject: ProfileId,
    path: Provisioner,
) -> ApiResult<Standing> {
    let mut conn = pool.acquire().await?;
    provision_conn(&mut conn, subject, path).await
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
