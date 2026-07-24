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
use crate::auth::SystemAdmin;
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
    /// The profile performing the disconnect.
    ///
    /// On the self-serve arm this equals the subject being unbound — the caller
    /// can only name their own principals. On the admin arm the two differ, and
    /// **distinguishing those two cases is the point of the audit event this
    /// field exists to feed**: "you unbound yourself" and "an operator unbound
    /// you" are different facts about the same row disappearing.
    pub actor: ProfileId,
}

/// Unbind a Slack principal: revoke the grant, then delete identity, secret and
/// intents in one transaction.
///
/// Idempotent — disconnecting an unlinked principal succeeds quietly, with both
/// booleans false, no intents swept, and [`IdpRevocation::NotAttempted`].
pub async fn disconnect_slack_principal(
    pool: &PgPool,
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
    let emitter = temper_substrate::writes::resolve_emitter(pool, req.actor, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut tx = pool.begin().await?;

    // 1. Read + decrypt the RT while it still exists (locks the vault row).
    let refresh_token = slack_grant_vault_service::take_refresh_token_for_disconnect(
        &mut tx,
        req.key,
        req.slack_principal_id,
    )
    .await?;

    // 2. Revoke. Best-effort in ExternalIdp mode; real and atomic in AS mode.
    let idp_revocation = match (&refresh_token, req.mode) {
        // No grant on file (or one we could not open — see
        // `take_refresh_token_for_disconnect`), so nothing was attempted.
        (None, _) => IdpRevocation::NotAttempted,
        (Some(rt), AuthMode::TemperAs) => {
            // The AS issued this token and stores it locally, so revocation is a
            // row update in THIS transaction — no network, no failure mode.
            //
            // Zero rows matched maps to `Failed`, NOT `NotAttempted`: we had a
            // grant and we DID attempt to revoke it, and the attempt found no
            // AS row. That is the silent-failure case the pinned-hash test below
            // exists to guard (a digest drift between Rust and the TypeScript
            // writer matches nothing and would otherwise report success).
            if revoke_as_refresh_token(&mut tx, rt).await? {
                IdpRevocation::Revoked
            } else {
                tracing::warn!(
                    principal = %req.slack_principal_id,
                    "slack disconnect: AS-mode revocation matched no refresh-token row. The local \
                     grant was destroyed; the AS row (if any) is still live."
                );
                IdpRevocation::Failed
            }
        }
        (Some(rt), AuthMode::ExternalIdp) => {
            match oauth_client::revoke_grant(&req.revoke_url, req.client_id, rt).await {
                Ok(()) => IdpRevocation::Revoked,
                Err(e) => {
                    // Principal + error only. Never the token.
                    tracing::warn!(
                        principal = %req.slack_principal_id,
                        error = %e,
                        "slack disconnect: IdP revocation failed; destroying the local grant \
                         anyway. The grant may remain live at the IdP until it expires — \
                         revoke it out-of-band if that matters."
                    );
                    IdpRevocation::Failed
                }
            }
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
        req.actor.uuid(),
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

/// Admin disconnect: unbind ANY principal, on behalf of an operator.
///
/// The authorization gate lives HERE, in the service, not in the HTTP handler.
/// That is the repo's `audit-handler-authz-drift` rule, and it is load-bearing
/// for this feature specifically: a `@temper disconnect` Slack surface is
/// already planned, and a gate that lives in the axum handler is one that the
/// next surface must remember to re-add. Enforcing it at the shared layer means
/// every surface inherits it by construction.
///
/// Mirrors `machine_registration_service::provision`, which gates the same way.
///
/// Note the router is NOT the gate: under `access_mode='open'` the gated router
/// admits everyone, so this check is the only thing standing between a
/// non-admin and unbinding someone else's account.
///
/// The operator is named by the [`SystemAdmin`] proof, and by nothing else. The
/// proof IS the check (admin-authz enclosure, spec §3): its presence in the
/// signature is the authorization requirement, so this function cannot be
/// reached without `require_system_admin` having run. There is no
/// `is_system_admin` call left to forget, and no bool for a future surface to
/// skip — which is what the handler-side comment about the planned `@temper
/// disconnect` surface was reaching for.
///
/// This is **Bucket 1** by the enclosure's classification: a pure
/// system-authority act, no resource or team scope, where the only legitimate
/// question is *"are you a system admin?"*. Do NOT widen it to `machine_authz`
/// or a team-owner arm — it is an operator act on someone else's identity.
///
/// `req.actor` is **overwritten** from `admin.actor()` rather than read. The
/// field still exists because the self-serve arm needs it and the ledger event
/// is fed from it, but on this arm it is caller-supplied input to an
/// authorization-adjacent decision, and the proof is the trustworthy spelling.
/// The old shape had to argue in prose that gating on the field and attributing
/// from the field kept them in agreement; deriving both from the proof makes the
/// agreement structural.
pub async fn admin_disconnect_slack_principal(
    pool: &PgPool,
    admin: &SystemAdmin,
    mut req: DisconnectRequest<'_>,
) -> ApiResult<DisconnectOutcome> {
    // The proof already answered "may you?" — before the decrypt and before any
    // DELETE, because it is minted before this function is entered at all.
    // What remains is "who?", and the proof answers that too.
    req.actor = admin.actor();

    tracing::info!(
        principal = %req.slack_principal_id,
        actor = %req.actor,
        "admin slack disconnect authorized"
    );

    disconnect_slack_principal(pool, req).await
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
    let token_hash = format!("{digest:x}");

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
mod tests {
    use super::*;
    // The production path no longer names `access_service` — the `&SystemAdmin` proof replaced
    // its `is_system_admin` call. The fixtures still do, to assert that a profile the test calls
    // an admin (or a non-admin) genuinely is one by the predicate's own definition.
    use crate::services::access_service;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use sqlx::PgPool;
    use temper_substrate::payloads::{AnchorTable, SlackPrincipalDisconnected};
    use temper_workflow::operations::Surface;
    use uuid::Uuid;

    // `key()` and `insert_profile()` exist in slack_grant_vault_service's test
    // module, but a `#[cfg(test)] mod tests` is private to its own module — they
    // are NOT reachable from here. Redefined locally, matching that module's
    // shape exactly (same key bytes, same full-UUID handle rationale).

    fn key() -> VaultKey {
        VaultKey::from_base64(&STANDARD.encode([3u8; 32])).unwrap()
    }

    /// Bind a principal to a profile, on a one-off pooled connection.
    ///
    /// `link_slack_principal` and `store_grant` take `&mut PgConnection` so the link callback can
    /// run BOTH in one transaction — the two writes are one fact, and as separate autocommits a
    /// process death between them left a user linked with an unrecoverable grant. These disconnect
    /// tests only need the rows to exist beforehand, so they arrange each on its own connection.
    async fn seed_link(pool: &PgPool, profile_id: Uuid, principal: &str) {
        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::slack_link_service::link_slack_principal(&mut conn, profile_id, principal)
            .await
            .expect("link");
    }

    /// Seal a grant for `principal` under `key`. The access token and TTL are fixed because no
    /// disconnect test varies them — what varies is the key (the rotation test) and the refresh
    /// token (the AS-mode tests, which match it against `kb_oauth_refresh_tokens`).
    async fn seed_grant(
        pool: &PgPool,
        key: &VaultKey,
        profile_id: Uuid,
        principal: &str,
        refresh_token: &str,
    ) {
        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::slack_grant_vault_service::store_grant(
            &mut conn,
            key,
            crate::services::slack_grant_vault_service::NewGrant {
                profile_id,
                slack_principal_id: principal,
                refresh_token,
                access_token: "at",
                access_ttl_secs: Some(3600),
            },
        )
        .await
        .expect("store");
    }

    /// Minimal profile insert. The handle is the FULL id: two UUIDv7s minted in
    /// the same millisecond share leading bytes, so a truncated handle collides
    /// on `kb_profiles_handle_key`.
    ///
    /// The `<handle>@web` emitter entity is **not** optional garnish. Since the
    /// disconnect began emitting a ledger event it resolves an emitter first
    /// (`temper_substrate::writes::resolve_emitter`), and that is a `fetch_one`
    /// with **no lazy creation** — a profile row with no emitter entity cannot
    /// author anything, and every disconnect against it fails with "no emitter
    /// entity `<handle>@web` for the resolved profile" before a single row is
    /// deleted. The marker comes from `Surface::ApiHttp` rather than a literal
    /// `"web"` so the fixture tracks the natural key instead of restating it.
    async fn insert_profile(pool: &PgPool) -> Uuid {
        let (id, handle) = insert_profile_without_emitter(pool).await;
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("{handle}@{}", Surface::ApiHttp.marker()))
        .execute(pool)
        .await
        .expect("seed the emitter entity the disconnect authors through");
        id
    }

    /// The bare profile row, with **no** emitter entity — the structurally
    /// incomplete shape `insert_profile` completes.
    ///
    /// Split out rather than inlined so the emitter-precondition test below
    /// exercises the *absence* of the entity without restating the profile
    /// insert, and so the two fixtures cannot drift on handle shape.
    ///
    /// Returns the handle too: `insert_profile` needs it to name the entity.
    async fn insert_profile_without_emitter(pool: &PgPool) -> (Uuid, String) {
        let id = Uuid::now_v7();
        let handle = format!("user-{id}");
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
            id,
            handle,
        )
        .execute(pool)
        .await
        .unwrap();
        (id, handle)
    }

    /// Make `profile` a system admin by the ONLY definition the code has.
    ///
    /// Under D11 `is_system_admin(p)` reads `kb_principal_governance` — a governance grant, no
    /// longer gating-team ownership. So the grant is what makes the admin; every other profile in
    /// these tests is a non-admin for free because it has no governance row. Mirrors `admin_fixture`
    /// in `tests/admin_ledger_test.rs`. The gating-team setup below is retained only for parity with
    /// that fixture's topology; it no longer carries authorization meaning.
    async fn make_system_admin(pool: &PgPool, profile: Uuid) {
        let slug = format!("gating-{}", Uuid::now_v7().simple());
        let team_id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
        )
        .bind(&slug)
        .fetch_one(pool)
        .await
        .expect("seed gating team");

        sqlx::query("UPDATE kb_system_settings SET gating_team_slug = $1 WHERE id = 1")
            .bind(&slug)
            .execute(pool)
            .await
            .expect("point gating_team_slug at the fixture team");

        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'owner'::team_role)",
        )
        .bind(team_id)
        .bind(profile)
        .execute(pool)
        .await
        .expect("make the fixture profile an OWNER of the gating team");

        // What actually confers admin-ness under D11: a governance grant.
        crate::test_support::grant_governance(pool, profile).await;

        // Asserted, not trusted: a fixture whose admin is not an admin makes the
        // non-admin test below pass for the wrong reason (nobody is an admin).
        assert!(
            access_service::is_system_admin(pool, ProfileId::from(profile))
                .await
                .unwrap(),
            "the fixture admin MUST satisfy the real gate",
        );
    }

    /// Mint the sealed `SystemAdmin` proof for a seeded admin — the admin arm requires one
    /// (admin-authz enclosure). Mirrors `machine_registration_service`'s helper of the same
    /// name: there is no test bypass on the seal, so the honest path is to run the real gate.
    ///
    /// Takes the profile id rather than seeding its own operator, unlike
    /// `test_support::system_admin_proof` — these tests assert on WHICH profile the ledger
    /// names, so the proof has to be minted for a caller the test already holds.
    async fn admin_proof(pool: &PgPool, admin: Uuid) -> crate::auth::SystemAdmin {
        let authed = crate::test_support::authenticated_profile_for(pool, admin).await;
        crate::auth::require_system_admin(pool, &authed)
            .await
            .expect("the fixture admin must mint a proof")
    }

    /// One `slack_principal_disconnected` row, as the ledger stores it.
    #[derive(Debug, sqlx::FromRow)]
    struct DisconnectEventRow {
        payload: serde_json::Value,
        references: serde_json::Value,
        producing_anchor_table: Option<String>,
        producing_anchor_id: Option<Uuid>,
    }

    /// Every `slack_principal_disconnected` row in the ledger, oldest first.
    ///
    /// Deliberately UNFILTERED by principal: the tests below assert on the total
    /// count, so a stray event attributed to some other principal is a failure
    /// here rather than something a `WHERE` clause hides.
    async fn disconnect_events(pool: &PgPool) -> Vec<DisconnectEventRow> {
        sqlx::query_as::<_, DisconnectEventRow>(
            r#"SELECT e.payload, e."references", e.producing_anchor_table, e.producing_anchor_id
                 FROM kb_events e
                 JOIN kb_event_types et ON et.id = e.event_type_id
                WHERE et.name = 'slack_principal_disconnected'
                ORDER BY e.id"#,
        )
        .fetch_all(pool)
        .await
        .expect("read the disconnect ledger")
    }

    /// The `idp_revocation` recorded on the one disconnect event in the ledger.
    ///
    /// Read back through `IdpRevocation`'s own `Deserialize` rather than
    /// compared as a string: that pins the plpgsql writer's spelling to the
    /// enum's `#[serde(rename_all = "snake_case")]` set, so a bind that wrote
    /// `Failed` or `"failed"` (quoted, the `to_string` mistake `as_str` exists to
    /// avoid) fails here instead of surfacing as an unreadable audit row.
    async fn sole_event_idp_revocation(pool: &PgPool) -> IdpRevocation {
        let events = disconnect_events(pool).await;
        assert_eq!(
            events.len(),
            1,
            "this assertion is vacuous without exactly one emitted event",
        );
        let raw = events[0]
            .payload
            .get("idp_revocation")
            .unwrap_or_else(|| panic!("payload has no idp_revocation: {}", events[0].payload))
            .clone();
        serde_json::from_value(raw)
            .expect("idp_revocation must be one of the enum's three wire spellings")
    }

    /// Read a UUID out of a jsonb string field, failing loudly on either a
    /// missing key or an unparseable value.
    ///
    /// Asserting on the parsed UUID rather than on the string keeps the tests
    /// indifferent to how the SQL writer spells a uuid into jsonb; what is being
    /// asserted is identity, not formatting.
    fn json_uuid(value: &serde_json::Value, key: &str) -> Uuid {
        let raw = value
            .get(key)
            .unwrap_or_else(|| panic!("payload has no {key:?} key: {value}"))
            .as_str()
            .unwrap_or_else(|| panic!("payload {key:?} is not a JSON string: {value}"));
        Uuid::parse_str(raw)
            .unwrap_or_else(|e| panic!("payload {key:?} is not a uuid: {raw} ({e})"))
    }

    /// Link `principal` to `subject`, then unbind it as `actor`.
    ///
    /// The shared arrangement for the ledger-event tests: everything they assert
    /// is downstream of one linked-then-unbound act, and the only axis that
    /// varies between them is whether `actor` equals `subject`.
    async fn link_then_disconnect(pool: &PgPool, principal: &str, subject: Uuid, actor: Uuid) {
        seed_link(pool, subject, principal).await;

        let out = disconnect_slack_principal(
            pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key(),
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "c",
                actor: ProfileId::from(actor),
            },
        )
        .await
        .expect("disconnect");

        assert!(
            out.was_linked,
            "the arrangement is only meaningful if a link was actually removed — an unlinked \
             principal emits nothing, and every assertion downstream would be vacuous",
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn disconnecting_an_unlinked_principal_is_a_quiet_no_op(pool: PgPool) {
        // There is no subject here — nothing is linked — so the actor is just
        // some profile making the (no-op) request.
        let actor = ProfileId::from(insert_profile(&pool).await);

        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: "slack:T0:UNEVER",
                key: &key(),
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/unused".to_string(),
                client_id: "c",
                actor,
            },
        )
        .await
        .expect("idempotent disconnect must not error");

        assert!(!out.was_linked);
        assert!(!out.grant_deleted);
        assert_eq!(out.intents_deleted, 0);
        assert_eq!(
            out.idp_revocation,
            IdpRevocation::NotAttempted,
            "no grant existed, so no revocation was ATTEMPTED — reporting a failed revocation \
             here is what made the CLI warn at users who had nothing vaulted",
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn disconnect_deletes_link_grant_and_intents_together(pool: PgPool) {
        let principal = "slack:T1:U1";
        let key = key();
        let profile_id = insert_profile(&pool).await;

        seed_link(&pool, profile_id, principal).await;
        seed_grant(&pool, &key, profile_id, principal, "rt").await;
        crate::services::slack_link_service::create_intent(
            &pool,
            principal,
            "verifier",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("intent");

        // Unreachable revoke URL: the IdP call must fail and must NOT block the unbind.
        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key,
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "c",
                // Self-serve shape: the actor unbinds their own principal.
                actor: ProfileId::from(profile_id),
            },
        )
        .await
        .expect("a failed IdP revoke must not fail the disconnect");

        assert!(out.was_linked);
        assert!(out.grant_deleted);
        assert_eq!(out.intents_deleted, 1);
        assert_eq!(
            out.idp_revocation,
            IdpRevocation::Failed,
            "the unreachable IdP must report an ATTEMPTED-and-failed revocation, distinct from \
             the no-grant case",
        );

        // The SAME observation, on the ledger. The return value tells the caller;
        // the event is what an offboarding auditor reads months later, and "is
        // the token actually dead?" is the question they need answered. A writer
        // that hardcoded a spelling, or that recorded the *intent* to revoke
        // rather than its outcome, would report a clean disconnect here.
        assert_eq!(
            sole_event_idp_revocation(&pool).await,
            IdpRevocation::Failed,
            "the ledger must record the revocation that actually failed, not a canned success",
        );

        let links: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider = 'slack' AND auth_provider_user_id = $1",
        )
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(links, 0, "the identity row must be gone");

        let grants: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        )
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(grants, 0, "the sealed grant must be destroyed, not flagged");

        let intents: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_slack_link_intents WHERE slack_principal_id = $1",
        )
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(intents, 0, "live intents must not survive a disconnect");

        // The profile itself is untouched — disconnect is not deactivation. Phase 2 moved
        // deactivation off the dropped `kb_profiles.is_active` onto `kb_principal_standing`, so the
        // check is now "no `deactivated` standing row was minted".
        let deactivated: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM kb_principal_standing WHERE profile_id = $1 AND state = 'deactivated')",
        )
        .bind(profile_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!deactivated, "disconnect is not deactivation");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn disconnecting_twice_is_not_an_error(pool: PgPool) {
        let principal = "slack:T2:U2";
        let key = key();
        let profile_id = insert_profile(&pool).await;
        seed_link(&pool, profile_id, principal).await;

        let req = || DisconnectRequest {
            slack_principal_id: principal,
            key: &key,
            mode: AuthMode::ExternalIdp,
            revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
            client_id: "c",
            // Self-serve shape, twice over — the same actor unbinds their own
            // principal, and the second call is the quiet no-op.
            actor: ProfileId::from(profile_id),
        };

        let first = disconnect_slack_principal(&pool, req())
            .await
            .expect("first");
        assert!(first.was_linked);
        let second = disconnect_slack_principal(&pool, req())
            .await
            .expect("second");
        assert!(!second.was_linked, "the second disconnect is a quiet no-op");

        // "Quiet" includes the ledger. `kb_events` is append-only and enforced so
        // by trigger, so a spurious second row cannot be retracted — only
        // outlived. Emission is therefore conditioned on a row having actually
        // been deleted, and this is the assertion that holds that condition in
        // place: a writer that emitted unconditionally would leave 2 here and
        // report a disconnect that never happened.
        assert_eq!(
            disconnect_events(&pool).await.len(),
            1,
            "only the disconnect that removed a link is an administrative act; the no-op second \
             call must add nothing to an append-only ledger",
        );
    }

    /// The audit record itself: exactly ONE event, naming who acted and who was unbound.
    ///
    /// **Exactly one, not at least one.** `kb_events` is append-only, so a
    /// duplicate is immortal and would double-count the act for every reader of
    /// the ledger; `>= 1` would pass against a writer that emitted twice.
    ///
    /// The both-NULL anchor is the **cognition firewall**. Admin acts ride
    /// `kb_events` but must be invisible to every anchor-scoped cognition
    /// reader; an event that acquired a producing anchor would surface an
    /// authority record inside ordinary trail reads.
    ///
    /// The `references` assertion is not decoration either — `list_by_subject`
    /// reads that axis, so an event with no `subject` reference is an audit
    /// record that cannot be found by the person it is about.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_disconnect_writes_exactly_one_event_naming_actor_and_subject(pool: PgPool) {
        let principal = "slack:T6:UEVENT";
        let profile_id = insert_profile(&pool).await;

        // Self-serve shape: the actor unbinds their own principal.
        link_then_disconnect(&pool, principal, profile_id, profile_id).await;

        let events = disconnect_events(&pool).await;
        assert_eq!(
            events.len(),
            1,
            "one unbind is one administrative act and must be exactly one ledger row",
        );
        let event = &events[0];

        assert_eq!(
            event
                .payload
                .get("slack_principal_id")
                .and_then(|v| v.as_str()),
            Some(principal),
            "the payload must name the principal that was unbound — the link row is gone, so this \
             is the only surviving record of WHICH binding was destroyed",
        );
        assert_eq!(
            json_uuid(&event.payload, "subject_id"),
            profile_id,
            "the subject is the profile that was unbound",
        );
        assert_eq!(
            json_uuid(&event.payload, "disconnected_by"),
            profile_id,
            "on the self-serve arm the actor IS the subject",
        );
        assert_eq!(
            event.payload.get("subject_table").and_then(|v| v.as_str()),
            Some(AnchorTable::Profiles.as_str()),
            "the subject is a profile: `AnchorTable` has no `kb_profile_auth_links` variant, and \
             the link row does not survive the act that describes it",
        );

        assert_eq!(
            event.producing_anchor_table, None,
            "the cognition firewall: an admin act carries no producing anchor",
        );
        assert_eq!(
            event.producing_anchor_id, None,
            "the cognition firewall: an admin act carries no producing anchor",
        );

        let references = event
            .references
            .as_array()
            .expect("`references` is a jsonb array");
        let subject_ref = references
            .iter()
            .find(|r| r.get("rel").and_then(|v| v.as_str()) == Some("subject"))
            .expect("a `subject` reference — the axis `list_by_subject` reads");
        let target = subject_ref
            .get("target")
            .expect("the subject reference carries a target");
        assert_eq!(
            target.get("kind").and_then(|v| v.as_str()),
            Some(AnchorTable::Profiles.as_str()),
        );
        assert_eq!(json_uuid(target, "id"), profile_id);
    }

    /// An admin unbinding SOMEONE ELSE records both identities, distinctly.
    ///
    /// This is the case the event exists for. "You unbound yourself" and "an
    /// operator unbound you" are different facts about the same row
    /// disappearing, and if `disconnected_by` and `subject_id` ever collapse
    /// onto one identity the ledger can no longer tell them apart — which is
    /// the entire value of the record.
    ///
    /// **Why this bites:** actor and subject are two DIFFERENT profiles, so a
    /// writer that derived `disconnected_by` from the deleted link row (the
    /// tempting shortcut — the profile is right there in `RETURNING`) would
    /// write the subject into both fields and fail here, while passing every
    /// self-serve test in this file, where the two are equal by construction.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_admin_disconnect_records_the_operator_not_the_subject(pool: PgPool) {
        let principal = "slack:T7:UADMIN";
        let subject = insert_profile(&pool).await;
        let operator = insert_profile(&pool).await;
        assert_ne!(
            subject, operator,
            "the fixture must exercise actor != subject"
        );

        link_then_disconnect(&pool, principal, subject, operator).await;

        let events = disconnect_events(&pool).await;
        assert_eq!(events.len(), 1);
        let payload = &events[0].payload;

        assert_eq!(
            json_uuid(payload, "disconnected_by"),
            operator,
            "the operator performed the act and must be named as its author",
        );
        assert_eq!(
            json_uuid(payload, "subject_id"),
            subject,
            "the subject is the profile that lost its binding, NOT the operator",
        );
    }

    /// The payload spells no key that `element_trail_*` matches on.
    ///
    /// `element_trail_node` and `element_trail_edge` match payload keys **by
    /// shape, with no event-type filter** — so an admin payload that happened to
    /// spell `resource_id`, `block_id`, `edge_id` or `owner` would leak an
    /// authority record into any cognition trail read gated only by
    /// `resources_visible_to`. Subjects are spelled `subject_table`/`subject_id`
    /// precisely to stay outside that match.
    ///
    /// This scans **every key the writer actually emitted** rather than checking
    /// that the four expected fields are present: the failure mode is an EXTRA
    /// key nobody thought to look for, and a test that only inspects the fields
    /// it expects is structurally incapable of seeing one.
    ///
    /// The sibling corpus scan in `tests/admin_ledger_test.rs`
    /// (`no_admin_payload_spells_a_trail_matched_key`) covers this type in name
    /// only — its seeder hardcodes `WHERE et.name = 'grant_created'`, so no
    /// `slack_principal_disconnected` row exists in that corpus and the scan
    /// matches zero rows for it. This test is the one that runs against a real
    /// row from the real writer.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_disconnect_payload_spells_no_trail_matched_key(pool: PgPool) {
        /// Keys `element_trail_node`/`element_trail_edge` match on by shape.
        /// Mirrors `BANNED_ADMIN_PAYLOAD_KEYS` in `tests/admin_ledger_test.rs`.
        const BANNED: &[&str] = &["resource_id", "block_id", "edge_id", "owner"];

        let principal = "slack:T8:UBANNED";
        let profile_id = insert_profile(&pool).await;
        link_then_disconnect(&pool, principal, profile_id, profile_id).await;

        let events = disconnect_events(&pool).await;
        assert_eq!(events.len(), 1, "the scan below is vacuous without a row");

        let payload = events[0]
            .payload
            .as_object()
            .expect("the payload is a jsonb object");
        assert!(
            !payload.is_empty(),
            "an empty payload would pass the scan below while carrying no audit value at all",
        );
        for key in payload.keys() {
            assert!(
                !BANNED.contains(&key.as_str()),
                "payload key {key:?} is matched by element_trail_* on shape alone, which would \
                 leak this authority record into an ordinary cognition read",
            );
        }
    }

    /// The SQL-built payload satisfies the Rust wire contract.
    ///
    /// The writer is plpgsql and the reader is a `#[derive(Deserialize)]`
    /// struct; nothing in either language connects them. A field renamed on one
    /// side, or a `subject_table` spelling `AnchorTable` cannot parse, compiles
    /// and ships fine and fails only when something first tries to read the
    /// event back. `verify_ledger_roundtrip` covers the seeded corpus; this
    /// proves the same contract for the row this service actually emits.
    ///
    /// **Why this bites:** `from_value` is strict about the enum — a
    /// `subject_table` of anything other than a spelling in `AnchorTable`'s
    /// `#[serde(rename)]` set fails to parse, as does a missing or
    /// non-uuid-shaped `disconnected_by`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_emitted_payload_deserializes_into_the_typed_struct(pool: PgPool) {
        let principal = "slack:T9:UTYPED";
        let profile_id = insert_profile(&pool).await;
        link_then_disconnect(&pool, principal, profile_id, profile_id).await;

        let events = disconnect_events(&pool).await;
        assert_eq!(events.len(), 1);

        let typed: SlackPrincipalDisconnected = serde_json::from_value(events[0].payload.clone())
            .expect(
                "the SQL-built payload must deserialize into the typed wire contract — a drift \
                 between the plpgsql writer and this struct is invisible until a reader fails",
            );

        assert_eq!(typed.subject_table, AnchorTable::Profiles);
        assert_eq!(typed.subject_id, profile_id);
        assert_eq!(typed.slack_principal_id, principal);
        assert_eq!(typed.disconnected_by, ProfileId::from(profile_id));
        // `link_then_disconnect` vaults no grant, so there was nothing to revoke.
        // The typed read is the strict half: `IdpRevocation` has no catch-all
        // variant, so any spelling the plpgsql writer invents fails the
        // `from_value` above outright rather than deserializing to a wrong-but-
        // parseable value.
        assert_eq!(typed.idp_revocation, IdpRevocation::NotAttempted);
    }

    /// The cross-language contract, pinned.
    ///
    /// `kb_oauth_refresh_tokens` rows are written by TypeScript
    /// (`packages/temper-cloud/src/oauth/mint.ts:85` — `createHash("sha256")
    /// .update(t).digest("hex")`), and revoked here by Rust. Nothing in the type
    /// system connects the two: if these digests ever disagree, the AS-mode
    /// revoke silently updates ZERO rows and reports success, leaving a live
    /// grant behind with no error anywhere.
    ///
    /// The expected value below was produced by the actual writer:
    ///   node -e 'const{createHash}=require("crypto");
    ///            console.log(createHash("sha256").update("as-refresh-token-sample")
    ///                        .digest("hex"))'
    /// Regenerate it the same way if this ever needs to change.
    #[test]
    fn the_as_token_hash_matches_what_typescript_writes() {
        use sha2::{Digest, Sha256};

        let digest = Sha256::digest(b"as-refresh-token-sample");
        assert_eq!(
            format!("{digest:x}"),
            "9d16e5d809978fbc29ae240d1b95273fc1ff0de968d8e4f98cadfa0b5802e199",
            "Rust's digest must equal Node's sha256 hex, or AS-mode revocation \
             matches no row and fails silently"
        );
    }

    /// AS mode revokes locally, in the same transaction — no network, no
    /// best-effort. This is why self-hosted gets strictly stronger semantics
    /// than the Auth0 path, so it must actually be exercised.
    #[sqlx::test(migrations = "../../migrations")]
    async fn as_mode_revokes_the_refresh_token_row_in_transaction(pool: PgPool) {
        let principal = "slack:T3:U3";
        let key = key();
        let profile_id = insert_profile(&pool).await;
        let refresh_token = "as-refresh-token-sample";

        seed_link(&pool, profile_id, principal).await;
        seed_grant(&pool, &key, profile_id, principal, refresh_token).await;

        // The AS's own row for that token, as the TypeScript writer would leave it.
        sqlx::query(
            r#"
            INSERT INTO kb_oauth_refresh_tokens (token_hash, client_id, claims, expires_at)
            VALUES ($1, $2, '{}'::jsonb, now() + interval '30 days')
            "#,
        )
        .bind("9d16e5d809978fbc29ae240d1b95273fc1ff0de968d8e4f98cadfa0b5802e199")
        .bind("slack-link-client")
        .execute(&pool)
        .await
        .expect("seed AS refresh token");

        // An unreachable revoke_url: if AS mode wrongly took the HTTP path, the
        // call would fail and idp_revoked would be false.
        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key,
                mode: AuthMode::TemperAs,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "slack-link-client",
                // Self-serve shape.
                actor: ProfileId::from(profile_id),
            },
        )
        .await
        .expect("disconnect");

        assert_eq!(
            out.idp_revocation,
            IdpRevocation::Revoked,
            "AS mode must revoke locally without touching the network",
        );

        // The successful-revocation pole of the ledger field. Paired with the
        // `Failed` assertion in `disconnect_deletes_link_grant_and_intents_
        // together` and the `NotAttempted` one in the rotated-key test, this
        // pins all three states to what actually happened — a writer that
        // recorded one constant would satisfy at most one of the three.
        assert_eq!(
            sole_event_idp_revocation(&pool).await,
            IdpRevocation::Revoked,
            "the ledger must record the revocation that actually succeeded",
        );

        let revoked_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT revoked_at FROM kb_oauth_refresh_tokens WHERE token_hash = $1",
        )
        .bind("9d16e5d809978fbc29ae240d1b95273fc1ff0de968d8e4f98cadfa0b5802e199")
        .fetch_one(&pool)
        .await
        .expect("read back");
        assert!(
            revoked_at.is_some(),
            "the AS row must be marked revoked, asserted on the row not the return value"
        );
    }

    /// **The key-rotation flag-day.** A grant sealed under one key must still be
    /// destroyable after `SLACK_VAULT_ENC_KEY` is rotated.
    ///
    /// Rotating the key makes every pre-rotation ciphertext unopenable by
    /// design. Before this fix the AEAD failure propagated out of
    /// `take_refresh_token_for_disconnect` via `?`, so the whole transaction
    /// aborted before COMMIT and **nothing was deleted** — both disconnect
    /// surfaces 500'd and unbound nothing, on a fleet where every grant is in
    /// that state, in the exact situation (key compromise) that motivates the
    /// rotation. The user is told to re-link, and re-link refuses to rebind to
    /// a different profile, so the stale identity row becomes unremovable.
    ///
    /// **Why this bites:** it seals under `key()` and disconnects under a
    /// DIFFERENT `VaultKey`, so the decrypt genuinely fails. Against the old `?`
    /// the call returns `Err` and every row assertion below finds its row still
    /// present. The assertions are on the three tables directly, not on the
    /// return value: a version that swallowed the error but skipped the deletes
    /// would pass a return-value-only check.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_grant_sealed_under_a_rotated_key_is_still_destroyed(pool: PgPool) {
        let principal = "slack:T4:UROTATED";
        let old_key = key();
        // The rotated key: same length, different bytes, so the AEAD open fails.
        let new_key = VaultKey::from_base64(&STANDARD.encode([9u8; 32])).unwrap();
        let profile_id = insert_profile(&pool).await;
        // Admin-shaped: post-rotation cleanup is the operator's repair path, so
        // the actor is a DIFFERENT profile from the subject being unbound. This
        // is the actor != subject case the audit event exists to distinguish.
        let operator = ProfileId::from(insert_profile(&pool).await);

        seed_link(&pool, profile_id, principal).await;
        seed_grant(
            &pool,
            &old_key,
            profile_id,
            principal,
            "rt-sealed-under-the-old-key",
        )
        .await;
        crate::services::slack_link_service::create_intent(
            &pool,
            principal,
            "verifier",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("intent");

        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &new_key,
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "c",
                actor: operator,
            },
        )
        .await
        .expect("an unopenable ciphertext must not brick the unbind lever");

        assert!(out.was_linked, "the identity row must have been removed");
        assert!(out.grant_deleted, "the sealed grant must be destroyed");
        assert_eq!(out.intents_deleted, 1);
        assert_eq!(
            out.idp_revocation,
            IdpRevocation::NotAttempted,
            "we never opened the token, so no revocation could be attempted",
        );
        assert_eq!(
            sole_event_idp_revocation(&pool).await,
            IdpRevocation::NotAttempted,
            "the ledger must say the grant was never reached — an auditor reading `failed` here \
             would chase a revocation that was never attempted, and one reading `revoked` would \
             believe a still-live grant is dead",
        );

        for (table, sql) in [
            (
                "identity",
                "SELECT count(*) FROM kb_profile_auth_links \
                 WHERE auth_provider = 'slack' AND auth_provider_user_id = $1",
            ),
            (
                "grant",
                "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
            ),
            (
                "intents",
                "SELECT count(*) FROM kb_slack_link_intents WHERE slack_principal_id = $1",
            ),
        ] {
            let n: i64 = sqlx::query_scalar(sql)
                .bind(principal)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                n, 0,
                "the {table} row must be gone even though the grant could not be opened",
            );
        }
    }

    /// AS mode with NO matching refresh-token row reports `Failed`, not
    /// `NotAttempted`.
    ///
    /// This is the silent-failure case: we held a grant, we tried to revoke it,
    /// and the UPDATE matched nothing (a digest drift from the TypeScript
    /// writer, or a token the AS never minted). Reporting `NotAttempted` would
    /// tell the operator "there was nothing to revoke", which is the opposite of
    /// the truth and suppresses the CLI warning that exists for exactly this.
    ///
    /// **Why this bites:** it seeds a grant but deliberately seeds NO
    /// `kb_oauth_refresh_tokens` row, so `revoke_as_refresh_token` returns
    /// `false`. A mapping that folded zero-rows into `NotAttempted` fails here.
    #[sqlx::test(migrations = "../../migrations")]
    async fn as_mode_matching_no_row_is_a_failure_not_a_no_op(pool: PgPool) {
        let principal = "slack:T5:UNOROW";
        let key = key();
        let profile_id = insert_profile(&pool).await;

        seed_link(&pool, profile_id, principal).await;
        seed_grant(
            &pool,
            &key,
            profile_id,
            principal,
            "a-token-the-as-never-minted",
        )
        .await;

        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key,
                mode: AuthMode::TemperAs,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "slack-link-client",
                // Self-serve shape.
                actor: ProfileId::from(profile_id),
            },
        )
        .await
        .expect("disconnect");

        assert_eq!(
            out.idp_revocation,
            IdpRevocation::Failed,
            "an attempted revocation that matched zero rows is a FAILURE — folding it into \
             NotAttempted is the silent-failure the pinned-hash test exists to guard",
        );
        assert!(out.grant_deleted, "the local grant is destroyed regardless");
    }

    /// How many slack link rows exist for `principal`. The survival check every
    /// refusal test below turns on.
    async fn link_count(pool: &PgPool, principal: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM kb_profile_auth_links \
             WHERE auth_provider = 'slack' AND auth_provider_user_id = $1",
        )
        .bind(principal)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The emitter precondition is INTENTIONAL, and it fails CLOSED.
    ///
    /// An actor with no `<handle>@web` entity cannot be attributed, and this
    /// module's rule that "the unbind must never be blocked" deliberately does
    /// NOT cover that case: the rule is about external flakiness (Slack down, a
    /// rotated vault key — see `a_grant_sealed_under_a_rotated_key_is_still_
    /// destroyed`, which unbinds anyway), not about the actor's own profile
    /// being structurally incomplete.
    ///
    /// An unattributable authority act is worse than a failed one. The
    /// alternative considered — emitting under the `system` entity — was
    /// REJECTED because the ledger's actor axis is derived FROM the emitter
    /// (`admin_ledger_service::fetch` joins `emitter_entity_id ->
    /// kb_entities.profile_id`), so a `system` fallback would file someone's
    /// unbind under `system` and make it unfindable by the person who performed
    /// it. A missing audit row is honest; a misattributed one lies.
    ///
    /// **Why this bites:** it asserts on the two TABLES, not on the returned
    /// error. A `system`-emitter fallback would return `Ok` and leave zero links
    /// (failing the survival assertion); a variant that resolved the emitter
    /// *inside* or *after* the transaction would return `Err` while having
    /// already deleted the link — passing an error-only check and failing here.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_actor_without_a_web_emitter_fails_the_disconnect_rather_than_unbinding_unattributed(
        pool: PgPool,
    ) {
        let principal = "slack:TA:UNOEMITTER";
        let subject = insert_profile(&pool).await;
        // Deliberately emitterless: a profile row that cannot author anything.
        let (actor, _handle) = insert_profile_without_emitter(&pool).await;

        seed_link(&pool, subject, principal).await;

        let err = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key(),
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "c",
                actor: ProfileId::from(actor),
            },
        )
        .await
        .expect_err("an unattributable disconnect must fail loudly, not proceed anonymously");

        assert!(
            matches!(err, ApiError::Internal(_)),
            "the emitter resolution failure surfaces as Internal, not as a quiet success: {err:?}",
        );

        assert_eq!(
            link_count(&pool, principal).await,
            1,
            "NOTHING may be unbound when the act cannot be attributed",
        );
        assert!(
            disconnect_events(&pool).await.is_empty(),
            "no emitter means no author, and `kb_events` is append-only — an event filed under a \
             fallback identity could never be corrected",
        );
    }

    // THERE IS NO "a non-admin is refused by the wrapper" TEST HERE, and its absence is the
    // enclosure landing rather than a gap.
    //
    // One lived here until the admin arm took a `&SystemAdmin`. It built a non-admin actor,
    // called the wrapper, and asserted `Forbidden` + link intact + no ledger row. That test is
    // no longer *expressible*: `SystemAdmin` is sealed (`auth/mod.rs` — private field, and
    // `require_system_admin` is the only constructor), so a non-admin cannot produce the
    // argument the call now requires. "Reaching this function without the gate" stopped being a
    // runtime outcome to assert on and became a compile error, which is the whole point.
    //
    // Its two halves both still have a home, and neither was dropped on the way:
    //   - the refusal itself → `tests/system_admin_proof_test.rs::non_admin_is_refused`, which
    //     pins the mint that every enclosed act now depends on, for all of them at once.
    //   - auth-before-writes (a refused caller leaves the link standing and writes no audit
    //     row) → `tests/e2e/tests/slack_link_test.rs::
    //     admin_disconnect_refuses_a_non_admin_and_leaves_the_link_intact`, which drives the
    //     real HTTP surface. That is now the honest layer for it: the refusal happens in the
    //     handler, before the service is entered, so a service-level test could only assert
    //     that un-run code wrote nothing.

    /// An admin passes the gate, and the ledger names the ADMIN as the author.
    ///
    /// The headline claim of `admin_disconnect_slack_principal` is that the operator who
    /// passed the gate is the one the audit record names, so the two cannot disagree about
    /// who acted. That is only provable through the wrapper, and only when actor != subject.
    ///
    /// **Why this bites:** a wrapper that authorized an admin but then handed the disconnect
    /// the subject (the tempting shortcut — it is the profile the principal belongs to) would
    /// succeed here and write the SUBJECT into `disconnected_by`, producing an audit trail in
    /// which every admin unbind looks self-serve.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_gated_wrapper_admits_an_admin_and_records_the_admin_as_the_author(pool: PgPool) {
        let principal = "slack:TB:UADMINOK";
        let subject = insert_profile(&pool).await;
        let admin = insert_profile(&pool).await;
        assert_ne!(subject, admin, "the fixture must exercise actor != subject");
        make_system_admin(&pool, admin).await;

        seed_link(&pool, subject, principal).await;

        let proof = admin_proof(&pool, admin).await;
        let out = admin_disconnect_slack_principal(
            &pool,
            &proof,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key(),
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "c",
                actor: ProfileId::from(admin),
            },
        )
        .await
        .expect("a system admin must be admitted");

        assert!(out.was_linked, "the admin arm must actually unbind");
        assert_eq!(link_count(&pool, principal).await, 0);

        let events = disconnect_events(&pool).await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            json_uuid(&events[0].payload, "disconnected_by"),
            admin,
            "the operator who passed the gate is the one the ledger must name",
        );
        assert_eq!(
            json_uuid(&events[0].payload, "subject_id"),
            subject,
            "the subject is the profile that lost its binding, NOT the operator",
        );
    }

    /// The PROOF names the operator — a stale `req.actor` cannot redirect the attribution.
    ///
    /// Before the enclosure this function both *gated on* and *attributed to* `req.actor`, a
    /// field of a caller-built struct, and its doc defended keeping those one field on the
    /// grounds that two spellings of one identity drift apart. The proof removes the choice
    /// rather than managing it: there is now exactly one source for the operator, and it is the
    /// one the gate minted.
    ///
    /// So this fixture hands the wrapper a request whose `actor` is the SUBJECT — the wrong
    /// spelling, and a non-admin besides. Under the old shape that was fatal in both directions
    /// at once: the gate read the field, so it would refuse; and had it admitted, it would have
    /// filed an operator act under the very profile being unbound. It must now simply be ignored.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_proof_names_the_operator_and_a_stale_request_actor_is_ignored(pool: PgPool) {
        let principal = "slack:TB:USTALEACTOR";
        let subject = insert_profile(&pool).await;
        let admin = insert_profile(&pool).await;
        make_system_admin(&pool, admin).await;
        seed_link(&pool, subject, principal).await;

        assert!(
            !access_service::is_system_admin(&pool, ProfileId::from(subject))
                .await
                .unwrap(),
            "the fixture subject must be a non-admin, or a read of the request field could still \
             pass the gate and the test would prove nothing",
        );

        let proof = admin_proof(&pool, admin).await;
        let out = admin_disconnect_slack_principal(
            &pool,
            &proof,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key(),
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "c",
                // Deliberately WRONG. The proof is the authority; this must not be read.
                actor: ProfileId::from(subject),
            },
        )
        .await
        .expect("the proof admits, whatever the request's actor field says");

        assert!(out.was_linked, "the admin arm must actually unbind");

        let events = disconnect_events(&pool).await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            json_uuid(&events[0].payload, "disconnected_by"),
            admin,
            "the ledger must name the admin the PROOF was minted for, never the request's actor \
             field — an operator act attributed to its own subject reads as self-serve",
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_reaper_sweeps_expired_and_consumed_intents_but_spares_live_ones(pool: PgPool) {
        use crate::services::slack_link_service::create_intent;

        // Live — must survive.
        create_intent(
            &pool,
            "slack:T1:ULIVE",
            "v-live",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("live intent");

        // Expired — must be swept.
        create_intent(
            &pool,
            "slack:T1:UEXP",
            "v-exp",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("expiring intent");
        sqlx::query("UPDATE kb_slack_link_intents SET expires_at = now() - interval '1 hour' WHERE slack_principal_id = $1")
            .bind("slack:T1:UEXP")
            .execute(&pool)
            .await
            .unwrap();

        // Consumed but not yet expired — must be swept (its purpose is spent).
        create_intent(
            &pool,
            "slack:T1:UUSED",
            "v-used",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("consumed intent");
        sqlx::query(
            "UPDATE kb_slack_link_intents SET consumed_at = now() WHERE slack_principal_id = $1",
        )
        .bind("slack:T1:UUSED")
        .execute(&pool)
        .await
        .unwrap();

        let swept = reap_expired_intents(&pool).await.expect("reap");
        assert_eq!(swept, 2, "expired and consumed rows are swept");

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT slack_principal_id FROM kb_slack_link_intents")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec!["slack:T1:ULIVE".to_string()]);
    }
}
