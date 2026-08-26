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
///      failed, a principal could be left `revoked` yet still an admin (`is_system_admin` reads
///      governance alone). Sharing one transaction makes the pair atomic — both land or neither.
pub async fn apply(pool: &PgPool, params: ApplyStandingParams) -> ApiResult<Standing> {
    let mut tx = pool.begin().await?;

    // Lock the subject's standing row for the life of the transaction, then read it. A concurrent
    // `apply` on the same subject blocks here until we commit.
    let current = load_locked(&mut tx, params.subject).await?;
    // (`&mut tx` deref-coerces to `&mut PgConnection` — the loader takes a bare connection so
    // both the pooled and transactional callers share one body.)

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
    // deployed CLI's "you already have a pending request" branch keys on it. A refusal drops `tx`
    // unmoved, so nothing is written.
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
    .fetch_one(&mut *tx)
    .await?;

    // The committer echoes back what it wrote. A disagreement means the SQL grew an opinion.
    debug_assert_eq!(committed.as_deref(), Some(resulting.as_str()));

    // §9 — Revoke and Deactivate demote, so "admin, but admission revoked" is never representable.
    // The invariant is maintained BY TRANSITION: a one-directional write (admission → governance),
    // never a read-time AND (`is_system_admin` reads governance alone). Fired unconditionally on
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

    tx.commit().await?;

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

    /// Seed a live refresh-token row the way the AS writes one, for `owner`.
    async fn live_refresh_token(pool: &PgPool, owner: Option<ProfileId>, token_hash: &str) {
        sqlx::query(
            "INSERT INTO kb_oauth_refresh_tokens \
               (token_hash, client_id, claims, expires_at, chain_expires_at, profile_id) \
             VALUES ($1, 'cli', '{}'::jsonb, now() + interval '30 days', \
                     now() + interval '90 days', $2)",
        )
        .bind(token_hash)
        .bind(owner.map(|o| *o))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn is_revoked(pool: &PgPool, token_hash: &str) -> bool {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT revoked_at FROM kb_oauth_refresh_tokens WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_one(pool)
        .await
        .unwrap()
        .is_some()
    }

    /// Whether the row claims it was retired BY ROTATION — a claim only the AS may make.
    async fn claims_rotation(pool: &PgPool, token_hash: &str) -> bool {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT rotated_at FROM kb_oauth_refresh_tokens WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_one(pool)
        .await
        .unwrap()
        .is_some()
    }

    /// Bring a principal to `Approved` through the seam, so the fixture is a real transition
    /// history rather than a hand-written row.
    async fn approved(pool: &PgPool, handle: &str, admin: ProfileId) -> ProfileId {
        let p = profile(pool, handle).await;
        for (act, authority, actor) in [
            (
                Act::Provision {
                    path: Provisioner::Saml,
                },
                ActorAuthority::Credential,
                None,
            ),
            (Act::Approve, ActorAuthority::Admin, Some(admin)),
        ] {
            apply(
                pool,
                ApplyStandingParams {
                    subject: p,
                    act,
                    actor,
                    authority,
                },
            )
            .await
            .unwrap();
        }
        p
    }

    /// Ending a principal's admission ends their live refresh chains, through the single standing
    /// writer, so every terminal path inherits it rather than each remembering to do it.
    ///
    /// The second principal is the regression arm, not decoration. De-provisioning one human must
    /// not touch another, and a `WHERE` clause that over-matched would still pass an
    /// assertion that only looked at the subject.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoking_a_principal_ends_their_live_refresh_chains_and_only_theirs(pool: PgPool) {
        let admin = profile(&pool, "chain-admin").await;
        let departing = approved(&pool, "chain-departing", admin).await;
        let remaining = approved(&pool, "chain-remaining", admin).await;

        live_refresh_token(&pool, Some(departing), "hash-departing-a").await;
        live_refresh_token(&pool, Some(departing), "hash-departing-b").await;
        live_refresh_token(&pool, Some(remaining), "hash-remaining").await;
        // A chain minted before an owner could be resolved. Un-endable here BY CONSTRUCTION, and
        // asserted so, because that limit is stated in the migration and must not quietly change.
        live_refresh_token(&pool, None, "hash-ownerless").await;

        apply(
            &pool,
            ApplyStandingParams {
                subject: departing,
                act: Act::Revoke {
                    reason: "left the company".into(),
                },
                actor: Some(admin),
                authority: ActorAuthority::Admin,
            },
        )
        .await
        .unwrap();

        assert!(is_revoked(&pool, "hash-departing-a").await);
        assert!(is_revoked(&pool, "hash-departing-b").await);
        assert!(
            !is_revoked(&pool, "hash-remaining").await,
            "revoking one principal must not touch another's chain"
        );
        assert!(
            !is_revoked(&pool, "hash-ownerless").await,
            "an ownerless chain is bounded by its absolute lifetime, not by this hook"
        );

        // This hook writes `revoked_at`, and so does the AS's rotation — which is exactly why the
        // AS also writes `rotated_at`, so that a token presented again after being spent can be
        // told from one an administrator ended (20260826000140). `revoked_at` has five writers and
        // only one is rotation, so that distinction is only worth anything if each of the other
        // four never makes the rotation claim. Asserted here, in the crate that owns this writer,
        // because the AS's own suite cannot see it: a stray `rotated_at` added to the UPDATE above
        // would turn every administrative revoke into a permanent, unfalsifiable report that the
        // user's credential had been stolen.
        assert!(
            !claims_rotation(&pool, "hash-departing-a").await,
            "an administrator's revoke must not present itself as a rotation"
        );
    }

    /// `Deactivate` is the other terminal, and it is reachable from states `Revoke` is not.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn deactivating_a_principal_also_ends_their_live_refresh_chains(pool: PgPool) {
        let admin = profile(&pool, "deact-admin").await;
        let subject = approved(&pool, "deact-subject", admin).await;
        live_refresh_token(&pool, Some(subject), "hash-deact").await;

        apply(
            &pool,
            ApplyStandingParams {
                subject,
                act: Act::Deactivate,
                actor: Some(admin),
                authority: ActorAuthority::Admin,
            },
        )
        .await
        .unwrap();

        assert!(is_revoked(&pool, "hash-deact").await);
    }

    /// An approval is not a terminal, so it must leave a live chain alone. A hook that fired on
    /// every transition would pass both tests above and log everyone out on approval; this is the
    /// arm that says so.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_non_terminal_transition_leaves_the_chain_alone(pool: PgPool) {
        let admin = profile(&pool, "approve-admin").await;
        let subject = profile(&pool, "approve-subject").await;
        apply(
            &pool,
            ApplyStandingParams {
                subject,
                act: Act::Provision {
                    path: Provisioner::Saml,
                },
                actor: None,
                authority: ActorAuthority::Credential,
            },
        )
        .await
        .unwrap();
        live_refresh_token(&pool, Some(subject), "hash-approve").await;

        apply(
            &pool,
            ApplyStandingParams {
                subject,
                act: Act::Approve,
                actor: Some(admin),
                authority: ActorAuthority::Admin,
            },
        )
        .await
        .unwrap();

        assert!(!is_revoked(&pool, "hash-approve").await);
    }

    /// The cross-language pin. `principal_may_refresh` (SQL, read by the Authorization Server) and
    /// this hook's terminal set (Rust) are two readings of one decision, in two languages, with
    /// nothing between them. This fails if they diverge in EITHER direction.
    ///
    /// The `match` is exhaustive on purpose: a new `Standing` variant does not silently inherit an
    /// answer here, it stops the build until someone decides which side it falls on. A new state
    /// added to the column's CHECK but not to the enum fails at `parse` instead.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_sql_refresh_gate_matches_the_rust_terminal_set(pool: PgPool) {
        fn rust_says_may_refresh(s: Standing) -> bool {
            match s {
                Standing::Denied | Standing::Requested | Standing::Approved => true,
                Standing::Revoked | Standing::Deactivated => false,
            }
        }

        for literal in ["denied", "requested", "approved", "revoked", "deactivated"] {
            let p = profile(&pool, &format!("pin-{literal}")).await;
            // Written directly rather than through the machine: this is a test OF the predicate,
            // and reaching every state through legal transitions would test the machine instead.
            sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, $2)")
                .bind(*p)
                .bind(literal)
                .execute(&pool)
                .await
                .unwrap();

            let sql_says: Option<bool> = sqlx::query_scalar("SELECT principal_may_refresh($1)")
                .bind(*p)
                .fetch_one(&pool)
                .await
                .unwrap();
            let standing = Standing::parse(literal).unwrap_or_else(|| {
                panic!("the column allows {literal:?} but Standing cannot parse it")
            });

            assert_eq!(
                sql_says,
                Some(rust_says_may_refresh(standing)),
                "principal_may_refresh and the Rust terminal set disagree about {literal:?}"
            );
        }

        // Absence denies, like every other standing predicate.
        let no_standing = profile(&pool, "pin-absent").await;
        let sql_says: Option<bool> = sqlx::query_scalar("SELECT principal_may_refresh($1)")
            .bind(*no_standing)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            sql_says,
            Some(false),
            "a principal with no standing row is not one we mint for"
        );
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
