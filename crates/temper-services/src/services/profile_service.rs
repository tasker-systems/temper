use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, PrincipalKind, Profile, ProfileAuthLink};
use temper_workflow::operations::Surface;

use crate::auth::HumanPrincipal;
use crate::auth_config::AuthMode;
use crate::error::{ApiError, ApiResult};

/// Maximum serialized size for the preferences JSON field (64KB).
const MAX_PREFERENCES_BYTES: usize = 65_536;

/// Validate that preferences JSON does not exceed the size limit.
pub fn validate_preferences_size(preferences: Option<&Value>) -> ApiResult<()> {
    if let Some(prefs) = preferences {
        let size = serde_json::to_string(prefs).map(|s| s.len()).unwrap_or(0);
        if size > MAX_PREFERENCES_BYTES {
            return Err(ApiError::BadRequest(format!(
                "preferences exceeds maximum size of {MAX_PREFERENCES_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

/// Generate a unique profile handle from a display name.
///
/// Slugifies the name (lowercase, non-alnum → dash, trim dashes), then appends
/// -2, -3, etc. if the handle already exists. `handle` is the substrate
/// addressing key (`slug` is §7-dissolved).
pub async fn generate_profile_handle(pool: &PgPool, display_name: &str) -> ApiResult<String> {
    let mut conn = pool.acquire().await?;
    generate_profile_handle_conn(&mut conn, display_name).await
}

/// Connection-taking twin of `generate_profile_handle`, for use inside a transaction.
pub(crate) async fn generate_profile_handle_conn(
    conn: &mut sqlx::PgConnection,
    display_name: &str,
) -> ApiResult<String> {
    let base: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive dashes (matches SQL backfill regex [^a-zA-Z0-9]+)
    let base: String = base
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() {
        "user".to_string()
    } else {
        base
    };

    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE handle = $1) as \"exists!: bool\"",
        &base,
    )
    .fetch_one(&mut *conn)
    .await?;

    if !exists {
        return Ok(base);
    }

    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        let exists = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE handle = $1) as \"exists!: bool\"",
            &candidate,
        )
        .fetch_one(&mut *conn)
        .await?;

        if !exists {
            return Ok(candidate);
        }
        suffix += 1;
    }
}

/// Resolve a profile from JWT claims.
///
/// Crate-private: the only ways in are `auth::authenticate_token` (the token path) and
/// `auth::resolve_federated_human` (the SAML path). A surface cannot reach the profile
/// *minting* path with claims it built itself — see the `auth` module doc.
///
/// Lookup order:
/// 1. Find an existing `kb_profile_auth_links` row by `(auth_provider, auth_provider_user_id)`.
/// 2. If found, load the linked profile.
/// 3. If not found, check whether any auth link shares the same email (reconciliation).
/// 4. If email matches an existing link, create a new auth link pointing to that profile.
/// 5. Otherwise, create a new profile and a new auth link.
///
/// `mode` is the instance's auth mode. It decides which machine registrations a machine token can
/// reach ([`crate::auth::machine_door::reachable_issuer`]); a human resolution ignores it.
pub(crate) async fn resolve_from_claims(
    pool: &PgPool,
    claims: &AuthClaims,
    mode: AuthMode,
) -> ApiResult<Profile> {
    match claims.principal_kind {
        PrincipalKind::Human => resolve_human_from_claims(pool, claims).await,
        PrincipalKind::Machine => resolve_machine_from_claims(pool, claims, mode).await,
    }
}

/// The machine-shape guard, shared by BOTH human doors.
///
/// One implementation, called from `resolve_human_from_claims` (resolve-or-provision) and
/// `resolve_existing_human_from_claims` (lookup-only), for the same reason
/// `auth::gate_resolved_profile` exists: the two paths differ only in whether they may
/// create, and a guard that lives in one copy per door drifts. It already had — the
/// lookup-only copy had lost the remediation sentence.
///
/// Either signal is disqualifying on its own — the provider tag (claims minted by the
/// machine seam) or the `@clients` subject shape (the Auth0 M2M convention, which no human
/// `sub` wears).
///
/// `door` names the caller in the log line only; the returned error is identical on both
/// paths and carries the remediation. The lookup-only caller collapses it into a generic
/// page, so the fuller text leaks nothing and gives an operator reading logs the actionable
/// command.
fn reject_machine_shaped(claims: &AuthClaims, door: &str) -> ApiResult<()> {
    let machine_provider = claims.provider == crate::auth::MACHINE_PROVIDER_TAG;
    let machine_shaped_id = claims.external_user_id.ends_with("@clients");
    if machine_provider || machine_shaped_id {
        // `external_user_id` is KEPT here, unlike the human sites: this branch is reached only for a
        // machine-shaped identity (machine provider tag or an `@clients` subject), so the value is a
        // machine client identifier, not the human OAuth `sub` §5 targets — not cross-linkable to a
        // person, and the one identifier this rejection has to name.
        tracing::warn!(
            external_user_id = %claims.external_user_id,
            provider = %claims.provider,
            door,
            "machine gate: rejected (machine-shaped identity on the {door})"
        );
        return Err(ApiError::Unauthorized(format!(
            "identity '{}' is machine-shaped and cannot resolve to a human profile. \
             A machine principal must be registered as a machine client: \
             temper admin machine provision --client-id <client_id> --label <label>, or \
             temper admin machine issue on an instance that runs its own authorization server",
            claims.external_user_id
        )));
    }
    Ok(())
}

/// Human path: link lookup → email reconcile → new profile.
///
/// Opens with a machine-shape guard, which is a *second* line behind
/// `auth::classify` (D-hardening). Classification decides which path a token takes;
/// this asserts at the dangerous action — minting a principal. The guard sits at the
/// top so it covers the lookup, reconcile AND create branches uniformly: even a
/// caller that bypasses classification entirely (a future surface, a test harness, a
/// direct service call) cannot walk a machine identity past the `kb_machine_clients`
/// registration gate by dressing it as a human. The machine path has a gate; the
/// human path has auto-provisioning — so a mislabeled machine is exactly the identity
/// that must never reach here.
pub(crate) async fn resolve_human_from_claims(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<Profile> {
    // 0: the machine-shape guard.
    reject_machine_shaped(claims, "human path")?;

    // 1 & 2: direct lookup by provider + external user id; load linked profile.
    // A verified sign-in refreshes the link's stored email + verification flag
    // (the self-heal path for rows that predate the email_verified column, and
    // for accounts verified at the provider after first sign-in).
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        refresh_link_verification(pool, &link, claims).await?;
        return get_by_id(pool, ProfileId::from(link.profile_id)).await;
    }

    // 3 & 4: email reconciliation — attach this provider to an existing profile.
    if let Some(profile) = reconcile_by_email(pool, claims).await? {
        return Ok(profile);
    }

    // 5: brand new profile + auth link, then provision its emitter entities and default context —
    // in ONE transaction, so an interrupted registration leaves no profile at all.
    //
    // These were two auto-committing statements on separate connections. The profile, its auth link
    // and its standing committed first; if the `acquire` or the provisioning then failed — pool
    // exhaustion, a transient blip — the request 500'd but the profile SURVIVED with zero
    // `kb_entities`. Step 1 above then found the auth link on every retry and returned early, so
    // provisioning never ran again: the profile could sign in, could link Slack (a plain INSERT
    // needing no emitter), and 500'd on any evented write, with no path back.
    // `20260716000020_backfill_legacy_profile_emitters.sql` exists because two production profiles
    // reached exactly that state; it repaired the instances without closing the path that makes them.
    //
    // This is the shared site for BOTH human doors — OAuth via `auth::authenticate` and SAML via
    // `auth::resolve_federated_human` — so the transaction covers both.
    let mut tx = pool.begin().await?;
    let (profile_id, handle) = create_new_profile_and_link(&mut tx, claims).await?;
    provision_profile_entities(&mut tx, profile_id, &handle).await?;
    tx.commit().await?;

    get_by_id(pool, ProfileId::from(profile_id)).await
}

/// Human path, LOOKUP-ONLY: link lookup → email reconcile → **refuse**.
///
/// `resolve_human_from_claims`'s steps 1-4 verbatim, minus step 5's create. It exists because
/// connecting Slack is NOT a registration route: `create_new_profile_and_link` INSERTs
/// `kb_profiles`, and a profile INSERT must never reach beyond itself — a birthed profile has
/// no standing row, so `has_system_access` is false and the auto-join mirror (materialized at
/// the standing committer, `20260923000010`) enrolls it nowhere. An enrollment at create would
/// be reach backed by no approved auth flow; today nothing enrolls a profile that was never
/// approved, and keeping this path create-free keeps that property structural.
///
/// Steps 3-4 (`reconcile_by_email`) are KEPT: they attach a link to an EXISTING profile,
/// creating nothing and enrolling nothing. Refusing them would make Slack-connect behave
/// differently from every other login for no safety gain.
///
/// Mirrors the machine arm, which is already lookup-or-reject (`resolve_machine_from_claims`).
/// `Ok(None)` means "no such profile" — the caller renders one indistinguishable refusal.
pub(crate) async fn resolve_existing_human_from_claims(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<Option<Profile>> {
    // 0: the machine-shape guard, THE SAME CALL as the create path's — not a copy of it.
    // A narrowing must not become a hole: a caller must not walk a machine identity past
    // the registration gate by choosing the lookup-only door.
    reject_machine_shaped(claims, "human lookup-only path")?;

    // 1 & 2: direct lookup by provider + external user id.
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        refresh_link_verification(pool, &link, claims).await?;
        return get_by_id(pool, ProfileId::from(link.profile_id))
            .await
            .map(Some);
    }

    // 3 & 4: email reconciliation — attaches to an EXISTING profile; mints nothing.
    if let Some(profile) = reconcile_by_email(pool, claims).await? {
        return Ok(Some(profile));
    }

    // 5: deliberately absent. No create branch.
    Ok(None)
}

/// Machine path: the registration gate (D2). Lookup-or-reject — there is no
/// create branch, because `machine_registration_service::provision` creates the
/// agent profile ahead of the machine's first call (D3).
///
/// This function is the ONLY machine-principal entry point for both temper-api and
/// temper-mcp, which is why the gate lives here and not in an Axum middleware (D4):
/// temper-mcp does not share temper-api's middleware stack, so a middleware gate would
/// drift. Rejections are specific (D7) — the caller has already proven it holds a valid,
/// correctly-audienced token, so naming the client id and the reason leaks nothing.
///
/// **The registration must be one this instance's issuer could have minted.** Each deployment
/// trusts one token issuer, and `mode` names it; a row recorded under the other issuer is
/// reachable by no legitimate token here, so a token that matches one is refused
/// ([`crate::auth::machine_door`]). The refusal names the way back, because the one honest way to
/// meet it is an instance that changed modes under a machine that was registered before.
async fn resolve_machine_from_claims(
    pool: &PgPool,
    claims: &AuthClaims,
    mode: AuthMode,
) -> ApiResult<Profile> {
    let client_id = claims.external_user_id.as_str();

    let Some(client) =
        crate::services::machine_client_service::lookup_by_client_id(pool, client_id).await?
    else {
        tracing::warn!(client_id, "machine gate: rejected (unregistered client)");
        // A client id in a namespace temper mints (a DCR client's `conn_`, say) cannot be
        // provisioned, so the provision hint would send the operator to a command that refuses it.
        let minted = crate::auth::secret::MINTED_CLIENT_ID_PREFIXES
            .iter()
            .any(|prefix| client_id.starts_with(prefix));
        return Err(ApiError::Unauthorized(if minted {
            format!(
                "machine client '{client_id}' is not registered with this instance, and its id is \
                 in a namespace temper mints, so it cannot be registered by hand"
            )
        } else if matches!(mode, AuthMode::TemperAs) {
            // An externally issued id can never authenticate here, so provision is no remedy.
            format!(
                "machine client '{client_id}' is not registered with this instance, which mints \
                 its own machine credentials. An administrator must issue one: \
                 temper admin machine issue --label <label>"
            )
        } else {
            format!(
                "machine client '{client_id}' is not registered with this instance. \
                 An administrator must run: temper admin machine provision --client-id {client_id} --label <label>"
            )
        }));
    };

    if let Some(revoked_at) = client.revoked_at {
        tracing::warn!(client_id, %revoked_at, "machine gate: rejected (revoked client)");
        return Err(ApiError::Unauthorized(format!(
            "machine client '{client_id}' was revoked at {}",
            revoked_at.to_rfc3339()
        )));
    }

    let reachable = crate::auth::machine_door::reachable_issuer(mode);
    if client.issuer != reachable {
        tracing::warn!(
            client_id,
            issuer = %client.issuer,
            %mode,
            "machine gate: rejected (registered under an issuer this instance does not trust)"
        );
        return Err(ApiError::Unauthorized(match mode {
            AuthMode::TemperAs => format!(
                "machine client '{client_id}' was registered under an external identity provider, which this instance no longer trusts; it mints its own machine credentials now. An administrator must issue a new one: temper admin machine issue --label <label>"
            ),
            AuthMode::ExternalIdp => format!(
                "machine client '{client_id}' is a temper-issued credential, which this instance cannot authenticate until it runs temper's own authorization server"
            ),
        }));
    }

    // Coarse liveness (D9). Failure to touch must not fail the request.
    if let Err(err) =
        crate::services::machine_client_service::touch_last_seen(pool, client.id).await
    {
        tracing::warn!(
            client_id,
            ?err,
            "machine gate: last_seen_at touch failed (ignored)"
        );
    }

    get_by_id(pool, ProfileId::from(client.profile_id)).await
}

/// Phase 1: direct lookup of an auth link by `(auth_provider, auth_provider_user_id)`.
async fn lookup_link_by_provider(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<Option<ProfileAuthLink>> {
    let link = sqlx::query_as!(
        ProfileAuthLink,
        r#"
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, email_verified,
               is_default, linked_at
          FROM kb_profile_auth_links
         WHERE auth_provider = $1
           AND auth_provider_user_id = $2
        "#,
        &claims.provider,
        &claims.external_user_id,
    )
    .fetch_optional(pool)
    .await?;

    Ok(link)
}

/// Refresh a link's stored email + verification on a verified sign-in: when the
/// incoming claims carry `email_verified: true` and the stored row disagrees
/// (unverified, or a different email), persist the provider's current truth.
/// No-op for unverified/missing claims — the flag never flips false→true here
/// without the provider's say-so, and never true→false at all (a later
/// unverified token doesn't un-verify an email the provider once verified).
async fn refresh_link_verification(
    pool: &PgPool,
    link: &ProfileAuthLink,
    claims: &AuthClaims,
) -> ApiResult<()> {
    if claims.email_verified != Some(true) {
        return Ok(());
    }
    let email_changed = link.email.as_deref() != Some(claims.email.as_str());
    if link.email_verified && !email_changed {
        return Ok(());
    }
    sqlx::query!(
        "UPDATE kb_profile_auth_links SET email = $2, email_verified = true WHERE id = $1",
        link.id,
        &claims.email as &str,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Phase 3 & 4: email reconciliation. Requires verification on BOTH sides: the
/// incoming claims must carry `email_verified: true`, and the matched stored link
/// must itself be verified — an unverified stored email is an attacker-controllable
/// claim (a pre-created account holding someone else's address must not capture
/// that person's first verified sign-in). When a verified existing link shares the
/// email, a new auth link for this provider is created pointing at that profile
/// and the profile is returned. Returns `None` otherwise (caller falls through to
/// new-profile creation).
///
/// The address matches **case-blind over ASCII letters only**: an exact-case match was a second
/// profile for anyone whose providers disagreed on case (`Alice@x.com` vs `alice@x.com`), and two
/// verified profiles on one address address nobody (`vw_invitee_invitations`). The fold is pinned
/// to the `C` collation because this match LINKS an identity to an existing profile: the database
/// collation's `lower()` also folds look-alikes (U+212A KELVIN SIGN to `k`), and a linking match
/// must never treat a different address as the same one. An exact-case link is preferred, then the
/// oldest, so the choice is deterministic; if more than one profile holds the address, a warning
/// says so (no address in it), since an operator has duplicates to merge.
async fn reconcile_by_email(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Option<Profile>> {
    if claims.email_verified != Some(true) {
        // `external_user_id` — the raw OAuth `sub` on the human path — is deliberately NOT emitted.
        // The machine-shape guard has already run, so this is a human identity in the pre-resolution
        // window; a `google-oauth2|…` value would join our exported traces to the same person in
        // unrelated systems. `provider` is not an end-user identifier and stays; the event remains
        // correlatable to its request through the root span's trace context. Decided 2026-08-01 (§5).
        tracing::warn!(
            provider = %claims.provider,
            "Skipping email reconciliation: email_verified is not true"
        );
        return Ok(None);
    }

    let reconciled_link = sqlx::query_as!(
        ProfileAuthLink,
        r#"
            SELECT id, profile_id, auth_provider, auth_provider_user_id, email, email_verified,
                   is_default, linked_at
              FROM kb_profile_auth_links
             WHERE lower(email COLLATE "C") = lower($1 COLLATE "C")
               AND email_verified
             ORDER BY (email = $1) DESC, linked_at, id
             LIMIT 1
            "#,
        &claims.email,
    )
    .fetch_optional(pool)
    .await?;

    let Some(existing) = reconciled_link else {
        return Ok(None);
    };

    let holders = sqlx::query_scalar!(
        r#"SELECT count(DISTINCT profile_id) AS "n!"
             FROM kb_profile_auth_links
            WHERE lower(email COLLATE "C") = lower($1 COLLATE "C")
              AND email_verified"#,
        &claims.email,
    )
    .fetch_one(pool)
    .await?;
    if holders > 1 {
        tracing::warn!(
            provider = %claims.provider,
            profiles = holders,
            "verified email held by more than one profile; linked to the exact-case or oldest holder"
        );
    }

    create_link_for_existing_profile(pool, existing.profile_id, claims).await?;

    Ok(Some(
        get_by_id(pool, ProfileId::from(existing.profile_id)).await?,
    ))
}

/// Phase 4: create a new (non-default) auth link for this provider pointing at
/// an existing profile.
async fn create_link_for_existing_profile(
    pool: &PgPool,
    profile_id: Uuid,
    claims: &AuthClaims,
) -> ApiResult<()> {
    let new_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
                INSERT INTO kb_profile_auth_links
                    (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
                VALUES ($1, $2, $3, $4, $5, $6, false, now())
                "#,
        new_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
        &claims.email as &str,
        claims.email_verified == Some(true),
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Phase 5a: create a brand-new profile and its default auth link. Returns the
/// new profile id and the generated handle (the latter feeds emitter provisioning).
///
/// Takes a connection, not the pool, so the caller can run it and Phase 5b in ONE transaction —
/// a profile that commits without its emitter entities is permanently stranded (see the caller).
/// Every write below must therefore stay on `conn`; reaching for the pool for any one of them
/// (most temptingly `standing_service::provision`) puts that write outside the transaction and
/// restores the bug in a form a rollback no longer covers.
async fn create_new_profile_and_link(
    conn: &mut sqlx::PgConnection,
    claims: &AuthClaims,
) -> ApiResult<(Uuid, String)> {
    let display_name = claims.email.split('@').next().unwrap_or("user").to_string();
    let handle = generate_profile_handle_conn(&mut *conn, &display_name).await?;

    let profile_id = Uuid::now_v7();

    sqlx::query!(
        r#"
        INSERT INTO kb_profiles
            (id, handle, display_name, email, preferences)
        VALUES ($1, $2, $3, $4, '{}')
        "#,
        profile_id,
        &handle,
        &display_name,
        &claims.email as &str,
    )
    .execute(&mut *conn)
    .await?;

    let auth_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
        VALUES ($1, $2, $3, $4, $5, $6, true, now())
        "#,
        auth_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
        &claims.email as &str,
        claims.email_verified == Some(true),
    )
    .execute(&mut *conn)
    .await?;

    // D11 — every door births Denied. THIS IS THE SHARED SITE for both human doors (SAML via
    // resolve_federated_human, OAuth via authenticate), and putting the call here rather than at
    // each caller is what removes the per-door constant that could be got wrong. Under the old
    // design the doors had to DIVERGE here, and a constant at this site would have been permissive
    // and silent — every signup born approved, with nothing to notice. Approval is a separate,
    // admin-authored act.
    // `provision_conn`, NOT `provision` — the pool-taking twin writes on a different connection
    // and would commit this standing row even when the surrounding transaction rolls back.
    crate::services::standing_service::provision_conn(
        &mut *conn,
        temper_core::types::ids::ProfileId::from(profile_id),
        temper_principal::Provisioner::OauthFirstLogin,
    )
    .await?;

    Ok((profile_id, handle))
}

/// Create an agent profile and its default machine auth link. Email is SQL NULL
/// (a machine has none); display name / handle derive from the client id.
///
/// Takes a connection so registration can run it inside a transaction. No longer
/// called from the authentication path — `provision` owns it now (D3).
pub(crate) async fn create_agent_profile_and_link(
    conn: &mut sqlx::PgConnection,
    client_id: &str,
) -> ApiResult<(Uuid, String)> {
    let display_name = format!("agent-{client_id}");
    let handle = generate_profile_handle_conn(&mut *conn, &display_name).await?;
    let profile_id = Uuid::now_v7();

    sqlx::query!(
        r#"
        INSERT INTO kb_profiles (id, handle, display_name, email, preferences)
        VALUES ($1, $2, $3, NULL, '{}')
        "#,
        profile_id,
        &handle,
        &display_name,
    )
    .execute(&mut *conn)
    .await?;

    let auth_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
        VALUES ($1, $2, $3, $4, NULL, false, true, now())
        "#,
        auth_link_id,
        profile_id,
        crate::auth::MACHINE_PROVIDER_TAG,
        client_id,
    )
    .execute(&mut *conn)
    .await?;

    Ok((profile_id, handle))
}

/// Create the dedicated profile a connection needs in order to own an emitter entity.
///
/// **Deliberately NOT [`create_agent_profile_and_link`]**, and the difference is the whole
/// point: that helper writes a `kb_profile_auth_links` row keyed `(auth0-m2m, client_id)`,
/// which is how a machine authenticates *to* temper. A connection is the opposite direction —
/// it is temper authenticating to a *remote* system. GitHub holds no temper token, so a
/// connection has no `client_id`, no auth link, and no `kb_machine_clients` row.
///
/// The profile exists for exactly one reason: `kb_events.emitter_entity_id` is NOT NULL and
/// FKs to `kb_entities`, which FKs to `kb_profiles`. No profile, no entity; no entity, no
/// emitted event. Email is SQL NULL — a connection has none.
pub(crate) async fn create_connection_profile(
    conn: &mut sqlx::PgConnection,
    provider: &str,
    name: &str,
) -> ApiResult<(Uuid, String)> {
    let display_name = format!("connection-{provider}-{name}");
    let handle = generate_profile_handle_conn(&mut *conn, &display_name).await?;
    let profile_id = Uuid::now_v7();

    sqlx::query!(
        r#"
        INSERT INTO kb_profiles (id, handle, display_name, email, preferences)
        VALUES ($1, $2, $3, NULL, '{}')
        "#,
        profile_id,
        &handle,
        &display_name,
    )
    .execute(&mut *conn)
    .await?;

    Ok((profile_id, handle))
}

/// Phase 5b: provision the per-surface emitter entities and the default context
/// a freshly created profile needs.
pub(crate) async fn provision_profile_entities(
    conn: &mut sqlx::PgConnection,
    profile_id: Uuid,
    handle: &str,
) -> ApiResult<()> {
    // Provision the per-surface emitter entities the write path resolves
    // (`<handle>@<surface>` — `writes::resolve_emitter`). The deleted synthesis
    // bootstrap used to create these; without them an auto-provisioned profile
    // could not emit events (every write would 500 on a missing emitter).
    //
    // Driven off `Surface::ALL` so a new surface variant provisions its emitter here by
    // construction. Existing profiles still need an additive backfill migration — see
    // `20260709000030_backfill_sdk_emitter_entities.sql` for the shape.
    //
    // Each emitter is still its own auto-commit statement, so two concurrent first-authenticated
    // requests for the same new profile both run this loop. `ON CONFLICT DO NOTHING` — inferring
    // the unique index added by `20260709000040_kb_entities_unique_profile_name.sql` — is what
    // makes that a no-op rather than a duplicate. It also makes provisioning re-runnable, so a
    // profile left half-provisioned by a failed request is repaired by calling this again.
    for surface in Surface::ALL {
        sqlx::query!(
            r#"
            INSERT INTO kb_entities (profile_id, name, metadata)
            VALUES ($1, $2, '{}'::jsonb)
            ON CONFLICT (profile_id, name) DO NOTHING
            "#,
            profile_id,
            format!("{handle}@{}", surface.marker()),
        )
        .execute(&mut *conn)
        .await?;
    }

    // Auto-provision a "default" context for the new profile.
    // Ignore conflict — if the profile somehow already has one, that's fine.
    sqlx::query!(
        r#"
        INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
        VALUES ($1, 'kb_profiles', $2, 'default', 'default')
        ON CONFLICT (owner_table, owner_id, slug) DO NOTHING
        "#,
        Uuid::now_v7(),
        profile_id,
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Load a profile by ID.
///
/// Deactivation is no longer a column here — the Level-1 authn gate
/// (`gate_resolved_profile`) reads it from `kb_principal_standing` (state
/// `'deactivated'`) since principal-admission Phase 2 dropped `is_active`.
/// Every `resolve_from_claims` path still routes through this loader.
pub async fn get_by_id(pool: &PgPool, id: ProfileId) -> ApiResult<Profile> {
    let profile = sqlx::query_as!(
        Profile,
        r#"
        SELECT id,
               display_name,
               handle AS "slug!",
               email,
               NULL::text AS avatar_url,
               preferences as "preferences: serde_json::Value",
               '{}'::jsonb AS "vault_config!: serde_json::Value",
               created,
               created AS "updated!"
          FROM kb_profiles
         WHERE id = $1
        "#,
        *id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("profile not found".to_string()))?;

    Ok(profile)
}

/// Update the caller's own mutable profile fields. Only provided (`Some`) values are written.
///
/// `vault_config` is intentionally not a parameter: it is substrate-dropped
/// (synthesized on read), so there is nothing to persist.
pub async fn update(
    pool: &PgPool,
    who: &HumanPrincipal,
    display_name: Option<&str>,
    preferences: Option<&Value>,
) -> ApiResult<Profile> {
    let id = who.profile_id();
    let current = get_by_id(pool, id).await?;

    let new_display_name = display_name.unwrap_or(&current.display_name);
    let new_preferences = preferences.unwrap_or(&current.preferences);

    sqlx::query!(
        r#"
        UPDATE kb_profiles
           SET display_name = $1,
               preferences  = $2
         WHERE id = $3
        "#,
        new_display_name,
        new_preferences as &Value,
        *id,
    )
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// List all auth links attached to the caller's own profile.
pub async fn list_auth_links(
    pool: &PgPool,
    who: &HumanPrincipal,
) -> ApiResult<Vec<ProfileAuthLink>> {
    let profile_id = who.profile_id();
    let links = sqlx::query_as!(
        ProfileAuthLink,
        r#"
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, email_verified,
               is_default, linked_at
          FROM kb_profile_auth_links
         WHERE profile_id = $1
         ORDER BY linked_at ASC
        "#,
        *profile_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(links)
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
