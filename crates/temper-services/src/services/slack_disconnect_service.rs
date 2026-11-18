//! Disconnect — unbind a Slack principal from a temper profile.
//!
//! The single chokepoint for every disconnect surface. Ordering is forced, not
//! chosen: the IdP's revocation endpoint takes the refresh token as a body
//! parameter, so the revoke must happen while the ciphertext still exists.
//!
//! The revoke is deliberately **best-effort**. Disconnect is the only unbind
//! lever in the system — and the remediation path for a mis-bound principal —
//! so gating it on third-party availability would be strictly worse than the
//! residual risk it closes. On failure we destroy the local copy anyway and log
//! a structured warning; we never persist the token to retry later, because
//! that would preserve the exact secret the user asked us to destroy.

use sqlx::PgPool;
use temper_core::types::slack::IdpRevocation;

use super::grant_crypto::VaultKey;
use super::slack_grant_vault_service;
use crate::auth::{HumanPrincipal, SystemAdmin};
use crate::auth_config::AuthMode;
use crate::error::{ApiError, ApiResult};
use crate::oauth_client;
use temper_core::types::ids::ProfileId;

/// What a disconnect actually did. Every field is an observation, not a promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectOutcome {
    /// Whether an identity row existed and was removed.
    pub was_linked: bool,
    /// Whether a vault row existed and was destroyed.
    pub grant_deleted: bool,
    /// How many link intents were swept for this principal.
    pub intents_deleted: i64,
    /// What happened to the grant at the IdP. [`IdpRevocation::Failed`] is not a
    /// failure of the *disconnect* — see the module docs — but it is distinct
    /// from [`IdpRevocation::NotAttempted`], which means there was no grant to
    /// revoke in the first place. Collapsing the two into a `bool` is what made
    /// the CLI warn about an unconfirmed revocation at users who had no grant.
    pub idp_revocation: IdpRevocation,
}

/// Inputs for a disconnect. A params struct because this crosses several
/// domain-related values and the codebase forbids growing the arg list.
#[derive(Debug)]
pub struct DisconnectRequest<'a> {
    pub slack_principal_id: &'a str,
    pub key: &'a VaultKey,
    pub mode: AuthMode,
    pub revoke_url: String,
    pub client_id: &'a str,
}

/// Unbind one of the caller's own Slack principals — the self-serve arm, where the actor is the
/// subject being unbound. A person's act: the `&HumanPrincipal` is what refuses a machine.
///
/// The caller names only its own principals; the handler derives them from the caller's own link
/// rows, so there is no input to forge.
pub async fn disconnect_own(
    pool: &PgPool,
    who: &HumanPrincipal,
    req: DisconnectRequest<'_>,
) -> ApiResult<DisconnectOutcome> {
    disconnect_slack_principal(pool, who.profile_id(), req).await
}

/// Unbind a Slack principal: revoke the grant, then delete identity, secret and
/// intents in one transaction. Crate-private: the two doors are [`disconnect_own`] (a person
/// unbinding themselves) and [`admin_disconnect_slack_principal`] (an operator, by proof).
///
/// Idempotent — disconnecting an unlinked principal succeeds quietly, with both
/// booleans false, no intents swept, and [`IdpRevocation::NotAttempted`].
///
/// `actor` is the profile performing the disconnect, and it is a **parameter
/// rather than a field of [`DisconnectRequest`]** — deliberately, and matching
/// `access_service::ReviewRequestParams`, which dropped its `reviewer` field for
/// the same reason. On the self-serve arm the actor equals the subject being
/// unbound (the caller can only name their own principals); on the admin arm the
/// two differ, and **distinguishing those cases is the point of the audit event
/// this value feeds** — "you unbound yourself" and "an operator unbound you" are
/// different facts about the same row disappearing. Keeping it out of the params
/// struct means the admin arm passes `admin.actor()` from its proof and a caller
/// has no field in which to supply an operator id that disagrees with who
/// actually authorized the act.
pub(crate) async fn disconnect_slack_principal(
    pool: &PgPool,
    actor: ProfileId,
    req: DisconnectRequest<'_>,
) -> ApiResult<DisconnectOutcome> {
    disconnect_as(pool, actor, false, req).await
}

/// [`disconnect_slack_principal`], saying whether the actor's authority is a system admin's. The
/// admin arm was decided on the pool (the proof), so it is asked again first on the transaction,
/// under the governance lock and before the vault row is locked, the token revoked or anything
/// deleted: an admin demoted since (or while this runs) disconnects nothing, and is refused `403`,
/// what a non-admin gets at the admin door. Lock order: governance, then the vault row, then the
/// refresh-token rows (AS mode) and the link rows. A demotion takes its subject's standing and
/// governance rows, then that subject's refresh tokens; it waits at the governance row for this
/// transaction before it reaches any token row.
async fn disconnect_as(
    pool: &PgPool,
    actor: ProfileId,
    on_admin_arm: bool,
    req: DisconnectRequest<'_>,
) -> ApiResult<DisconnectOutcome> {
    // Resolved on the pool, before the transaction opens: the emitter is a read
    // of the actor's entity, not part of the mutation, and the reference sink
    // (`access_service::grant_capability`) resolves it the same way.
    //
    // THIS IS A HARD PRECONDITION, AND DELIBERATELY SO. `resolve_emitter` is a
    // `fetch_one` with no lazy creation, so an actor with no `<handle>@web`
    // entity 500s here having unbound NOTHING. That sits uneasily beside this
    // module's rule that the unbind must never be blocked — but the rule is
    // about EXTERNAL flakiness (Slack down, a rotated vault key), not about the
    // actor's own profile being structurally incomplete. A missing emitter means
    // the caller cannot be attributed, and an unattributable authority act is
    // worse than a failed one: the alternative considered was emitting under the
    // `system` entity, which was rejected because the ledger's actor axis is
    // derived FROM the emitter (`admin_ledger_service::fetch` joins
    // `emitter_entity_id -> kb_entities.profile_id`), so a system fallback would
    // file the act under `system` and make it unfindable by the person who
    // performed it. A missing audit row is honest; a misattributed one lies.
    //
    // Unreachable today, on every arm, and the shape of that is worth recording:
    // `AuthUser` resolves only Human and Machine principals, both of which run
    // `provision_profile_entities`; the two shapes that lack `@web` by design —
    // CONNECTION profiles (`<handle>@webhook` only, minted inline by
    // `connection_service`) and `system` — are exactly the two that cannot hold
    // a bearer token. `disconnect_me` adds a second shield by deriving
    // principals from the caller's own link rows, so a linkless caller never
    // reaches this line.
    //
    // WHAT WOULD BREAK IT: a surface that lets a connection profile be the
    // actor. The `@temper disconnect` Slack surface anticipated in
    // `admin_disconnect_slack_principal`'s doc below is the live candidate — if
    // it routes through a connection profile, this call is the thing that
    // breaks, and it must provision or reject before it gets here.
    let emitter = temper_substrate::writes::resolve_emitter(pool, actor, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut tx = pool.begin().await?;
    if on_admin_arm && !crate::auth::is_system_admin_locked(&mut tx, actor).await? {
        return Err(ApiError::Forbidden);
    }

    // 1. Read + decrypt the RT while it still exists (locks the vault row).
    let refresh_token = slack_grant_vault_service::take_refresh_token_locked(
        &mut tx,
        req.key,
        req.slack_principal_id,
    )
    .await?;

    // 2. Revoke. Best-effort in ExternalIdp mode; real and atomic in AS mode.
    let idp_revocation = match &refresh_token {
        // No grant on file (or one we could not open — see
        // `take_refresh_token_locked`), so nothing was attempted.
        None => IdpRevocation::NotAttempted,
        Some(rt) => {
            revoke_at_idp(
                &mut tx,
                rt,
                &IdpRevokeTarget {
                    mode: req.mode,
                    revoke_url: &req.revoke_url,
                    client_id: req.client_id,
                },
                req.slack_principal_id,
            )
            .await?
        }
    };

    // 3. Destroy the secret.
    let grant_deleted = sqlx::query!(
        "DELETE FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        req.slack_principal_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    // 4. Destroy the identity binding.
    //
    // The DELETE and its `slack_principal_disconnected` event ride ONE
    // transaction — this one — via the SQL chokepoint `_admin_slack_disconnected`
    // (migrations/20260719000020). A Rust-side "delete, then insert an event"
    // pair would be two statements that a mid-flight failure can split; the
    // function makes the unbind and its record inseparable by construction.
    //
    // The subject profile is captured inside the function by `DELETE ...
    // RETURNING profile_id`, and it HAS to be: the auth-link row does not
    // survive the act, so there is nothing left to look up afterwards. It is
    // also why the event's subject is the profile rather than the link —
    // `AnchorTable` has no `kb_profile_auth_links` variant, and its `as_str` has
    // no catch-all arm, so the link row is not addressable as an anchor at all.
    // The Slack principal string rides in the payload instead.
    //
    // Emission is suppressed when nothing was linked. A disconnect of an
    // already-unbound principal is a quiet no-op, not an administrative act, and
    // `kb_events` is append-only — a spurious event cannot be retracted, only
    // outlived. Mirrors `access_service::delete_grant`, which emits only when a
    // grant row was actually removed.
    //
    // Correlation self-roots: there is no sibling event for this act to fuse
    // with, and the SQL fn's `p_correlation` defaults NULL.
    // `idp_revocation` is step 2's outcome, carried into the record on purpose. The unbind commits
    // whatever Slack said (the revoke is best-effort by design), so without it a disconnect
    // performed while Slack was unreachable would be indistinguishable, on the ledger, from a clean
    // one — and "is the token actually dead?" is the question an offboarding auditor most needs
    // answered. `as_str` rather than serde: the bind is a bare `text`, and `to_string` would quote it.
    let was_linked = sqlx::query_scalar!(
        r#"SELECT _admin_slack_disconnected($1,$2,$3,$4) AS "was_linked!""#,
        emitter.uuid(),
        req.slack_principal_id,
        actor.uuid(),
        idp_revocation.as_str(),
    )
    .fetch_one(&mut *tx)
    .await?;

    // 5. Sweep intents.
    //
    // Load-bearing, and NOT hygiene. The link design closes URL-theft with two
    // guarantees: D9 never issues a URL to a linked user, and rebind is refused.
    // A disconnect removes BOTH at once — so any intent minted before it and
    // still inside its TTL becomes a live, consumable *first-link* URL for a
    // now-unlinked principal. Leaving these rows reopens the hole disconnect is
    // supposed to be safe against.
    let intents_deleted = sqlx::query!(
        "DELETE FROM kb_slack_link_intents WHERE slack_principal_id = $1",
        req.slack_principal_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;

    tx.commit().await?;

    tracing::info!(
        principal = %req.slack_principal_id,
        was_linked,
        grant_deleted,
        intents_deleted,
        ?idp_revocation,
        "slack disconnect completed"
    );

    Ok(DisconnectOutcome {
        was_linked,
        grant_deleted,
        intents_deleted,
        idp_revocation,
    })
}

/// Where a Slack refresh token is revoked, and as which client.
#[derive(Debug, Clone, Copy)]
pub struct IdpRevokeTarget<'a> {
    pub mode: AuthMode,
    pub revoke_url: &'a str,
    pub client_id: &'a str,
}

/// Revoke one refresh token: locally in AS mode, in the caller's transaction, or at the IdP's
/// revoke endpoint, best-effort. The one revoke both a disconnect and a person erasure run
/// (`erasure_slack_revocation_service`). `label` names the grant in logs — a principal, or a
/// retry row's id — and is never the token.
///
/// Never returns [`IdpRevocation::NotAttempted`]: a token was in hand, so an attempt was made.
pub async fn revoke_at_idp(
    conn: &mut sqlx::PgConnection,
    refresh_token: &str,
    target: &IdpRevokeTarget<'_>,
    label: &str,
) -> ApiResult<IdpRevocation> {
    Ok(match target.mode {
        AuthMode::TemperAs => {
            // The AS issued this token and stores it locally, so revocation is a
            // row update in THIS transaction — no network, no failure mode.
            //
            // Zero rows matched maps to `Failed`, NOT `NotAttempted`: we had a
            // grant and we DID attempt to revoke it, and the attempt found no
            // AS row. That is the silent-failure case the pinned-hash test below
            // exists to guard (a digest drift between Rust and the TypeScript
            // writer matches nothing and would otherwise report success).
            if revoke_as_refresh_token(conn, refresh_token).await? {
                IdpRevocation::Revoked
            } else {
                tracing::warn!(
                    grant = %label,
                    "slack revoke: AS-mode revocation matched no refresh-token row. The local \
                     grant is destroyed; the AS row (if any) is still live."
                );
                IdpRevocation::Failed
            }
        }
        AuthMode::ExternalIdp => {
            match oauth_client::revoke_grant(target.revoke_url, target.client_id, refresh_token)
                .await
            {
                Ok(()) => IdpRevocation::Revoked,
                Err(e) => {
                    // The label and the error only. Never the token. The error text is
                    // bounded — it quotes the IdP response, not our config.
                    let err_text = e.to_string();
                    tracing::warn!(
                        grant = %label,
                        error = %crate::error::bounded(&err_text),
                        "slack revoke: IdP revocation failed. The grant may remain live at the \
                         IdP until it expires."
                    );
                    IdpRevocation::Failed
                }
            }
        }
    })
}

/// Admin disconnect: unbind ANY principal, on behalf of an operator.
///
/// The authorization requirement lives HERE, in the service's signature, not in
/// any one surface. That is the repo's `audit-handler-authz-drift` rule, and it
/// is load-bearing for this feature specifically: a `@temper disconnect` Slack
/// surface is already planned, and a gate written into the axum handler would be
/// one that surface must remember to re-add. A [`SystemAdmin`] parameter is one
/// it cannot compile without — every surface inherits it by construction.
///
/// The proof IS the check (admin-authz enclosure, spec §3): holding one is only
/// possible via `require_system_admin`, so this function cannot be reached
/// without the gate having run. There is no `is_system_admin` call left to
/// forget and no bool for a future surface to skip. Mirrors
/// `machine_registration_service::rebind`, the enclosure's closest precedent —
/// **not** `provision`, which is a scoped `machine_authz` gate on a bare
/// `ProfileId`.
///
/// This is **Bucket 1** by the enclosure's classification: a pure
/// system-authority act, no resource or team scope, where the only legitimate
/// question is *"are you a system admin?"*. Do NOT widen it to `machine_authz`
/// or a team-owner arm — it is an operator act on someone else's identity.
///
/// Note the router is NOT the gate: since D11 `has_system_access` reads
/// `kb_principal_standing` (approved), so the gated router denies unapproved
/// profiles — but the proof is still the only thing standing between a non-admin
/// and unbinding someone else's account.
///
/// The operator is named by the proof and by nothing else — there is no `actor`
/// field for a caller to supply, so an operator id that disagrees with who
/// authorized the act is unrepresentable rather than overwritten. The old shape
/// had to argue in prose that gating on a request field and attributing from
/// that same field kept the two in agreement; taking both from the proof makes
/// the agreement structural.
pub async fn admin_disconnect_slack_principal(
    pool: &PgPool,
    admin: &SystemAdmin,
    req: DisconnectRequest<'_>,
) -> ApiResult<DisconnectOutcome> {
    // The proof already answered "may you?" — before the decrypt and before any
    // DELETE, because it is minted before this function is entered at all.
    // What remains is "who?", and the proof answers that too.
    tracing::info!(
        principal = %req.slack_principal_id,
        actor = %admin.actor(),
        "admin slack disconnect authorized"
    );

    disconnect_as(pool, admin.actor(), true, req).await
}

/// Revoke the stored grant a re-link is about to overwrite. Call it in the callback's transaction,
/// after the link row is confirmed and before `store_grant`. **It revokes in AS mode only.**
///
/// A re-link replaces the vaulted refresh token (`store_grant` upserts on the principal), and the
/// vault is the only place that token's plaintext lives. Once it is overwritten, nothing can revoke
/// it. Disconnect revokes before it deletes for exactly that reason. The superseded token may be
/// live: a user who completes two outstanding link URLs supersedes the live grant the first one
/// stored, and Auth0 answers `invalid_grant` for a misconfigured client as well as for a dead grant
/// (see `mint_access_token`'s `GrantRejected` arm). Which mode decides the asymmetry:
///
/// - **AS mode: revoked here.** Its revocation is a local update to that one token's hash row, in
///   this transaction. It cannot reach the replacement grant (a new chain with a different hash),
///   and it commits or rolls back with the re-link.
/// - **ExternalIdp mode: deliberately NOT revoked, even though it may be live.** A revoke call is
///   not safe after the new consent. With Auth0's "Refresh Token Revocation Deletes Grant" enabled,
///   revoking one refresh token revokes every token for the same user, application and audience,
///   and that includes the replacement this callback just obtained, so a re-link would brick
///   itself. The overwrite still destroys temper's only copy of the old token, so temper can never
///   present it again. The supersede is logged so the orphaned grant is at least visible.
///
/// A token equal to the replacement is never revoked. A missing grant is a no-op. An unopenable one
/// is a no-op too, logged by [`slack_grant_vault_service::take_refresh_token_locked`], whose row
/// lock this takes.
pub async fn revoke_superseded_grant(
    conn: &mut sqlx::PgConnection,
    key: &VaultKey,
    mode: AuthMode,
    slack_principal_id: &str,
    replacement_refresh_token: &str,
) -> ApiResult<()> {
    let Some(superseded) =
        slack_grant_vault_service::take_refresh_token_locked(conn, key, slack_principal_id).await?
    else {
        return Ok(());
    };
    if superseded == replacement_refresh_token {
        return Ok(());
    }
    match mode {
        AuthMode::TemperAs => {
            let revoked = revoke_as_refresh_token(conn, &superseded).await?;
            if revoked {
                tracing::info!(
                    principal = %slack_principal_id,
                    "slack re-link: superseded AS refresh token revoked"
                );
            } else {
                // Usually benign: a grant the AS refused is often already revoked there, which is
                // why the user is re-linking. It is also what a missing AS row looks like (reaped,
                // or a hash drift — which the pinned-hash test guards), so it is not silent.
                tracing::info!(
                    principal = %slack_principal_id,
                    "slack re-link: superseded AS refresh token matched no unrevoked AS row"
                );
            }
        }
        AuthMode::ExternalIdp => {
            tracing::info!(
                principal = %slack_principal_id,
                "slack re-link: superseded refresh token overwritten without IdP revocation \
                 (ExternalIdp; revoking could also revoke the replacement grant)"
            );
        }
    }
    Ok(())
}

/// Revoke a temper-AS refresh token locally, in the caller's transaction.
///
/// The AS stores only `sha256(token)` as lowercase hex (`packages/temper-cloud/
/// src/oauth/mint.ts:85` — `createHash("sha256").update(t).digest("hex")`), so
/// we reproduce that digest and flip `revoked_at`. Idempotent, matching the
/// TypeScript `revokeRefreshToken` (`flow.ts:179`).
async fn revoke_as_refresh_token(
    tx: &mut sqlx::PgConnection,
    refresh_token: &str,
) -> ApiResult<bool> {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(refresh_token.as_bytes());
    let token_hash = hex::encode(digest);

    let affected = sqlx::query!(
        r#"
        UPDATE kb_oauth_refresh_tokens
           SET revoked_at = now()
         WHERE token_hash = $1
           AND revoked_at IS NULL
        "#,
        token_hash
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    Ok(affected > 0)
}

/// Sweep link intents that can no longer serve a purpose.
///
/// An intent is dead once it has expired or been consumed — a consumed row's
/// nonce is single-use and already burnt. Until this existed nothing ever
/// deleted from `kb_slack_link_intents`, so every principal that ever linked
/// left its PKCE verifier on disk indefinitely.
///
/// Live rows (unconsumed and unexpired) are spared: they are challenges a user
/// may still be about to click.
pub async fn reap_expired_intents(pool: &PgPool) -> ApiResult<i64> {
    let swept = sqlx::query!(
        r#"
        DELETE FROM kb_slack_link_intents
         WHERE consumed_at IS NOT NULL
            OR expires_at <= now()
        "#
    )
    .execute(pool)
    .await?
    .rows_affected() as i64;

    if swept > 0 {
        tracing::info!(swept, "slack link intents reaped");
    }
    Ok(swept)
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
