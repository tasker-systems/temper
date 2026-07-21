//! Shared authentication + authorization orchestration for both surfaces.
//!
//! The gate *sequence* lives here exactly once. temper-api and temper-mcp both
//! call these functions and map [`AuthzError`] to their own transport; neither
//! re-implements the ordering. Adding a future gate is one edit here, enforced
//! on every surface.
//!
//! **The seam owns principal construction.** A surface hands in a verified token
//! and gets back an [`AuthenticatedProfile`]; it never builds an `AuthClaims`, and
//! nothing here accepts one. That is the whole enforcement mechanism: `AuthClaims`
//! is still a public type a surface *can* construct, but with `authenticate`,
//! `classify`, `Principal` and `resolve_from_claims` all crate-private, a forged one
//! has nowhere to go. It is inert rather than forbidden.
//!
//! This closes the level below the one `normalize.rs` closed. PR #384 made
//! *classification* total, so no surface could say "unrecognized ⇒ human" — but each
//! surface still hand-built its own human `AuthClaims`, and they disagreed:
//! temper-api ran a three-rung email ladder, temper-mcp set `email: ""` and
//! auto-provisioned. Any surface that can construct an `AuthClaims` can construct a
//! `PrincipalKind::Human`, which is precisely the asymmetry that made #384's bug
//! asymmetric. One constructor, one ladder (`email.rs`), one answer per token.
//!
//! Entry points:
//! - [`authenticate_token`] — the token path. Verified [`RawJwtClaims`] + the raw
//!   token ⇒ authenticated profile. Both surfaces' every authed request.
//! - [`authenticate_token_existing_only`] — the same token path, LOOKUP-ONLY: it
//!   refuses rather than provisioning. For flows that authenticate a human but are
//!   not registration routes (the Slack account-link callback). Same seam, same
//!   Level 1 gate; it differs only in never minting a profile.
//! - [`resolve_federated_human`] — the federated path. An assertion already
//!   authenticated out-of-band (SAML/HMAC, no JWT) ⇒ resolved-or-JIT'd profile.
//! - [`require_system_access`] — Level 2, consuming proof of Level 1.
//!
//! Two levels form a typestate chain:
//! 1. `authenticate` (crate-private, reached only via [`authenticate_token`]) — resolve
//!    the profile + `is_active`. Runs on every authed request on both surfaces.
//!    Yields [`AuthenticatedProfile`].
//! 2. [`require_system_access`] — consumes proof of Level 1, adds the access gate.
//!    Runs on the gated tier of both surfaces. Yields [`SystemAuthorized`].

use sqlx::PgPool;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, AuthenticatedProfile, PrincipalKind, Profile};

mod email;
mod normalize;
pub mod secret;
pub use normalize::{RawJwtClaims, MACHINE_PROVIDER_TAG};
// Crate-private on purpose: classification is a decision the seam makes, not one a
// surface is shown. A surface that can see `Principal` can pattern-match its way back
// to hand-building the human arm — the exact drift this module exists to prevent.
pub(crate) use normalize::{classify, Principal};

use crate::error::{ApiError, ApiResult};
use crate::services::profile_service;
use crate::state::AppState;

/// The reason an authn/authz gate refused a request. Each surface maps these to
/// its own transport (HTTP status / rmcp error); the variants are the shared
/// vocabulary of *why*, never the words on the wire.
#[derive(Debug)]
pub enum AuthzError {
    /// The token is machine-shaped but not coherently so — classification (crate-private,
    /// see `normalize.rs`) refused it. Carries the reason (already logged by the seam) so a
    /// surface can decide how much of it to say on the wire; neither says any of it.
    Refused(&'static str),
    /// The human email ladder fell off its bottom rung: no `email` claim, no cached
    /// auth link, and `/userinfo` did not answer. Distinct from [`Self::ProfileResolution`]
    /// because nothing was resolved — we could not even name the human, so no write
    /// was attempted.
    EmailResolution(ApiError),
    /// `resolve_from_claims` failed (DB error, missing link data, etc.).
    ProfileResolution(ApiError),
    /// The `has_system_access` gate check itself failed (DB error) — distinct
    /// from a clean `SystemAccessDenied`, so surfaces can keep the pre-seam
    /// "failed to check system access" diagnostic instead of collapsing it into
    /// the resolve-failure message.
    AccessCheck(ApiError),
    /// The resolved profile is soft-deleted (`is_active == false`).
    Deactivated { profile_id: uuid::Uuid },
    /// The profile is not an approved member of the gating team.
    /// Carries the id so a surface can build its own denial payload.
    SystemAccessDenied { profile_id: uuid::Uuid },
}

/// **The token path.** A verified JWT ⇒ an authenticated, active profile.
///
/// The one entry point for both surfaces' authed requests. The surface verifies the
/// signature (each has its own audience) and decodes into [`RawJwtClaims`]; from
/// there the seam owns everything — classification, the human email ladder, claim
/// construction, and the Level 1 gates. `token` is the raw bearer, needed only for
/// the ladder's `/userinfo` rung.
///
/// The machine arm **must not** run the email ladder: an M2M principal has no email
/// and no `/userinfo` to ask, so a ladder on that path would be an authentication
/// failure dressed as a lookup. That ordering is load-bearing, not incidental — see
/// `machine_token_authenticates_without_running_the_email_ladder`.
pub async fn authenticate_token(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthenticatedProfile, AuthzError> {
    let claims = claims_from_token(state, raw, token).await?;

    authenticate(&state.pool, &claims).await
}

/// The classify + human-email-ladder block, shared by both token entry points.
///
/// Private, and it stays private: it is the *only* constructor of a human `AuthClaims` on
/// the token path, so both entry points are guaranteed to agree on classification, provider
/// name and the email ladder. A surface reaches it only by handing in a `RawJwtClaims` it
/// verified — never an `AuthClaims` it authored.
///
/// The machine arm must NOT run the email ladder: an M2M principal has no email and no
/// `/userinfo` to ask, so a ladder on that path would be an authentication failure dressed
/// as a lookup. That ordering is load-bearing — see
/// `machine_token_authenticates_without_running_the_email_ladder`.
async fn claims_from_token(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthClaims, AuthzError> {
    match classify(raw) {
        Principal::Machine(machine) => Ok(machine),
        Principal::Refuse(why) => {
            tracing::warn!(sub = %raw.sub, why, "rejected: unclassifiable machine-shaped token");
            Err(AuthzError::Refused(why))
        }
        Principal::Human => {
            let (email, email_verified) = email::resolve_email_from_claims(state, raw, token)
                .await
                .map_err(AuthzError::EmailResolution)?;
            Ok(AuthClaims {
                principal_kind: PrincipalKind::Human,
                provider: state.config.auth_provider_name.clone(),
                external_user_id: raw.sub.clone(),
                email,
                email_verified,
                exp: raw.exp,
                iat: raw.iat,
            })
        }
    }
}

/// **The token path, LOOKUP-ONLY.** A verified JWT ⇒ an *existing* profile, or a refusal.
///
/// Same seam as [`authenticate_token`] — it takes raw claims and the bearer, and builds the
/// `AuthClaims` itself via `claims_from_token`, so no surface can hand it forged ones. It
/// differs in exactly one way: it never provisions.
///
/// Used by the Slack account-link callback. Connecting Slack is not a registration route: the
/// profile INSERT that auto-provisioning performs fires `trg_sync_system_membership`, which in
/// `open` mode joins EVERY auto-join team. See the T2 spec, D3.
///
/// Level 1's post-resolution gate is **not** skipped: this shares `gate_resolved_profile`
/// with `authenticate`, so an inactive profile is refused here exactly as it is on the login
/// path. Level 2 ([`require_system_access`]) remains the caller's to apply, identical to the
/// login path.
pub async fn authenticate_token_existing_only(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthenticatedProfile, AuthzError> {
    let claims = claims_from_token(state, raw, token).await?;

    let profile = profile_service::resolve_existing_human_from_claims(&state.pool, &claims)
        .await
        .map_err(AuthzError::ProfileResolution)?
        .ok_or_else(|| {
            tracing::info!(sub = %raw.sub, "slack link: refused (no existing temper profile)");
            AuthzError::Refused("no existing temper profile for this identity")
        })?;

    gate_resolved_profile(profile, &claims)
}

/// **The federated path.** An identity asserted by a trusted peer, not a token.
///
/// Called by the internal SAML reconcile endpoint, whose assertion the co-deployed
/// Authorization Server has *already* validated server-to-server (HMAC over the body,
/// per `require_internal_signature`) before the token is minted. There is no JWT here
/// and nothing to classify: the caller is trusted, and this only resolves-or-JITs the
/// profile the minted token will later resolve to. `provider` must be the server's
/// configured provider name — never a payload field — so both paths land on the same
/// profile.
///
/// The trust assumption is exactly "the caller authenticated the assertion". It is not
/// a hole in the machine gate: #384's machine-shape guard at the top of
/// `resolve_human_from_claims` still covers this path, so an assertion carrying an
/// `@clients`-suffixed `external_user_id` is refused here as it is everywhere else.
pub async fn resolve_federated_human(
    pool: &PgPool,
    provider: &str,
    external_user_id: &str,
    email: &str,
    email_verified: Option<bool>,
) -> ApiResult<Profile> {
    let claims = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: provider.to_string(),
        external_user_id: external_user_id.to_string(),
        email: email.to_string(),
        email_verified,
        // exp/iat are unused by resolve_from_claims; supply zero rather than inventing a clock.
        exp: 0,
        iat: 0,
    };

    profile_service::resolve_from_claims(pool, &claims).await
}

/// Level 1 — authentication. Verified+normalized claims → a resolved, active profile.
///
/// Runs on **every** authenticated request on **both** surfaces, reached only through
/// [`authenticate_token`]. Crate-private: a surface cannot hand this function claims it
/// built itself, which is what makes a forged `AuthClaims` inert (module doc).
pub(crate) async fn authenticate(
    pool: &PgPool,
    claims: &AuthClaims,
) -> Result<AuthenticatedProfile, AuthzError> {
    let profile = profile_service::resolve_from_claims(pool, claims)
        .await
        .map_err(AuthzError::ProfileResolution)?;

    gate_resolved_profile(profile, claims)
}

/// Level 1's post-resolution tail: every gate that applies *after* a profile is resolved,
/// whichever door resolved it.
///
/// Factored out so [`authenticate`] (resolve-or-provision) and
/// [`authenticate_token_existing_only`] (lookup-only) cannot drift: a narrowing that skipped
/// a gate would be a hole, not a narrowing. It is deliberately the *whole* tail — resolution
/// is the only thing the two paths do differently, so anything after it belongs here, and a
/// future Level 1 gate added here binds both paths by construction.
///
/// Takes the profile by value and returns the [`AuthenticatedProfile`] so that constructing
/// one without passing the gate requires going out of your way.
fn gate_resolved_profile(
    profile: Profile,
    claims: &AuthClaims,
) -> Result<AuthenticatedProfile, AuthzError> {
    if !profile.is_active {
        return Err(AuthzError::Deactivated {
            profile_id: profile.id,
        });
    }

    Ok(AuthenticatedProfile {
        profile,
        claims: claims.clone(),
    })
}

/// Proof that a profile passed **both** levels: authenticated *and*
/// system-authorized. Only obtainable from [`require_system_access`], which
/// only accepts an [`AuthenticatedProfile`] — so the type makes it impossible
/// to run Level 2 without having passed Level 1.
#[derive(Debug)]
pub struct SystemAuthorized(pub AuthenticatedProfile);

/// Level 2 — system authorization. Consumes proof of Level 1, adds the
/// gating-team access gate. Runs on the gated tier of both surfaces.
pub async fn require_system_access(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> Result<SystemAuthorized, AuthzError> {
    let has_access = crate::services::access_service::has_system_access(
        pool,
        ProfileId::from(authed.profile.id),
    )
    .await
    .map_err(AuthzError::AccessCheck)?;

    if !has_access {
        return Err(AuthzError::SystemAccessDenied {
            profile_id: authed.profile.id,
        });
    }

    Ok(SystemAuthorized(authed.clone()))
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    use crate::config::ApiConfig;
    use crate::state::{AppState, JwksKeyStore};

    /// An `AppState` whose IdP is *unroutable on purpose*. Port 1 refuses instantly,
    /// so the userinfo rung of the email ladder can never succeed — any test that
    /// passes against this state proved it resolved without reaching the network,
    /// and any test that fails against it proves the ladder ran off its bottom rung.
    fn state(pool: PgPool) -> AppState {
        let config = ApiConfig {
            database_url: "unused".to_string(),
            auth: crate::auth_config::AuthConfig {
                issuer: "http://127.0.0.1:1".to_string(),
                jwks_url: "http://127.0.0.1:1/jwks".to_string(),
                audience: "test-audience".to_string(),
                mode: crate::auth_config::AuthMode::ExternalIdp,
            },
            auth_provider_name: "test-provider".to_string(),
            cors_origins: vec![],
            port: 0,
            enable_swagger: false,
            internal_reconcile_secret: None,
            embed_dispatch_secret: None,
            vercel_connect: None,
            slack_link: None,
            slack_mint_secret: None,
        };
        AppState::new(
            pool,
            JwksKeyStore::new("http://127.0.0.1:1/jwks".to_string()),
            config,
        )
    }

    /// A human token as Auth0 mints it *without* our email Action: a bare `sub`.
    fn human_raw(sub: &str, email: Option<&str>) -> RawJwtClaims {
        RawJwtClaims {
            sub: sub.to_string(),
            email: email.map(str::to_string),
            email_verified: None,
            azp: None,
            gty: None,
            exp: 9999,
            iat: 1111,
        }
    }

    /// A `client_credentials` token in the shape Auth0 actually mints.
    fn machine_raw(client_id: &str) -> RawJwtClaims {
        RawJwtClaims {
            sub: format!("{client_id}@clients"),
            email: None,
            email_verified: None,
            azp: Some(client_id.to_string()),
            gty: Some("client-credentials".to_string()),
            exp: 9999,
            iat: 1111,
        }
    }

    async fn profile_count(pool: &PgPool) -> i64 {
        sqlx::query_scalar!(r#"SELECT count(*) as "count!: i64" FROM kb_profiles"#)
            .fetch_one(pool)
            .await
            .expect("count profiles")
    }

    /// Seed a profile whose auth link already carries a cached email — the state a
    /// returning human is in after their first sign-in through temper-api.
    async fn seed_linked_human(pool: &PgPool, sub: &str, email: &str) -> uuid::Uuid {
        let profile_id = uuid::Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, $2, $2, $3, '{}')",
            profile_id,
            format!("human-{sub}"),
            email,
        )
        .execute(pool)
        .await
        .expect("seed profile");
        sqlx::query!(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at) \
             VALUES ($1, $2, 'test-provider', $3, $4, true, true, now())",
            uuid::Uuid::now_v7(),
            profile_id,
            sub,
            email,
        )
        .execute(pool)
        .await
        .expect("seed auth link");
        profile_id
    }

    /// The deliberate closure: an email-less, link-less human token no longer
    /// auto-provisions a junk `email: ''` profile on the surface that used to skip the
    /// ladder (temper-mcp). Both surfaces now run the one ladder, so both refuse — and
    /// refusal must be *before* any write, not after.
    #[sqlx::test(migrations = "../../migrations")]
    async fn emailless_unlinked_human_is_refused_without_provisioning(pool: PgPool) {
        let before = profile_count(&pool).await;

        let err = authenticate_token(
            &state(pool.clone()),
            &human_raw("auth0|nobody", None),
            "tok",
        )
        .await
        .expect_err("an email-less, link-less human must not authenticate");

        assert!(
            matches!(err, AuthzError::EmailResolution(ApiError::Unauthorized(_))),
            "expected EmailResolution(Unauthorized), got {err:?}"
        );
        assert_eq!(
            profile_count(&pool).await,
            before,
            "a refused token must provision nothing"
        );
    }

    /// The path that keeps every returning MCP human working: no `email` claim on the
    /// token, but a cached `kb_profile_auth_links` row from an earlier sign-in. Rung 2
    /// answers, so the (unroutable) userinfo rung is never reached.
    #[sqlx::test(migrations = "../../migrations")]
    async fn human_without_email_claim_resolves_from_cached_link(pool: PgPool) {
        let expected = seed_linked_human(&pool, "auth0|returning", "returning@example.test").await;

        let authed = authenticate_token(
            &state(pool.clone()),
            &human_raw("auth0|returning", None),
            "tok",
        )
        .await
        .expect("a linked human resolves from the cached email");

        assert_eq!(authed.profile.id, expected);
        assert_eq!(authed.claims.email, "returning@example.test");
        assert_eq!(
            authed.claims.principal_kind,
            temper_core::types::PrincipalKind::Human
        );
    }

    /// The machine path must never touch the email ladder — an M2M token has no email
    /// and no `/userinfo` to ask. The unroutable issuer is the assertion: if the ladder
    /// ran, this call would fail against port 1 instead of authenticating.
    #[sqlx::test(migrations = "../../migrations")]
    async fn machine_token_authenticates_without_running_the_email_ladder(pool: PgPool) {
        let expected = register_machine(&pool, "ladder-free-client").await;

        let authed = authenticate_token(
            &state(pool.clone()),
            &machine_raw("ladder-free-client"),
            "tok",
        )
        .await
        .expect("a registered machine authenticates with no email resolution");

        assert_eq!(authed.profile.id, expected);
        assert_eq!(
            authed.claims.principal_kind,
            temper_core::types::PrincipalKind::Machine
        );
        assert_eq!(authed.claims.email, "", "a machine has no email");
    }

    /// `Principal::Refuse` reaches the surfaces as `AuthzError::Refused` — the closed
    /// sum's refusal is now a value of the *seam's* error vocabulary, not something each
    /// surface re-derives from a classification it can see.
    #[sqlx::test(migrations = "../../migrations")]
    async fn refused_classification_surfaces_as_refused(pool: PgPool) {
        let before = profile_count(&pool).await;

        // Machine-shaped subject with no `client_credentials` grant: incoherent.
        let raw = human_raw("abc123@clients", None);
        let err = authenticate_token(&state(pool.clone()), &raw, "tok")
            .await
            .expect_err("an unclassifiable machine-shaped token must be refused");

        assert!(
            matches!(err, AuthzError::Refused(why) if why.contains("machine-shaped")),
            "expected Refused, got {err:?}"
        );
        assert_eq!(
            profile_count(&pool).await,
            before,
            "a refused token must provision nothing"
        );
    }

    /// The federated (SAML) path JITs a profile from an assertion the caller already
    /// authenticated server-to-server. No token, no ladder — the email is asserted.
    #[sqlx::test(migrations = "../../migrations")]
    async fn federated_human_resolves_from_asserted_identity(pool: PgPool) {
        let profile = resolve_federated_human(
            &pool,
            "test-provider",
            "saml|jane",
            "jane@example.test",
            Some(true),
        )
        .await
        .expect("federated resolve");

        assert_eq!(profile.email.as_deref(), Some("jane@example.test"));

        // Idempotent: the same assertion resolves to the same profile.
        let again = resolve_federated_human(
            &pool,
            "test-provider",
            "saml|jane",
            "jane@example.test",
            Some(true),
        )
        .await
        .expect("federated resolve (repeat)");
        assert_eq!(again.id, profile.id);
    }

    /// The narrowing, at the seam: an unknown identity is refused rather than provisioned.
    /// The profile-count assertion is the load-bearing half — a regression that provisions
    /// and *then* errors would still return Err, so refusing is not the same as not minting.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_only_token_path_refuses_unknown_identity_without_provisioning(pool: PgPool) {
        let before = profile_count(&pool).await;

        let err = authenticate_token_existing_only(
            &state(pool.clone()),
            &human_raw("auth0|stranger", Some("stranger@example.test")),
            "tok",
        )
        .await
        .expect_err("an unknown identity must not authenticate on the lookup-only path");

        assert!(
            matches!(err, AuthzError::Refused(why) if why.contains("no existing temper profile")),
            "expected Refused, got {err:?}"
        );
        assert_eq!(
            profile_count(&pool).await,
            before,
            "the lookup-only path must provision nothing (D3)"
        );
    }

    /// Level 1's post-resolution gate is NOT skipped by the narrowing. Without the shared
    /// `gate_resolved_profile` call this returns Ok and hands a deactivated profile to the
    /// Slack callback — a lookup-only path that resolves an inactive account is a hole.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_only_token_path_still_refuses_a_deactivated_profile(pool: PgPool) {
        let id = seed_linked_human(&pool, "auth0|gone", "gone@example.test").await;
        sqlx::query("UPDATE kb_profiles SET is_active = false WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("deactivate");

        let err = authenticate_token_existing_only(
            &state(pool.clone()),
            &human_raw("auth0|gone", None),
            "tok",
        )
        .await
        .expect_err("a deactivated profile must be refused on the lookup-only path too");

        assert!(
            matches!(err, AuthzError::Deactivated { profile_id } if profile_id == id),
            "expected Deactivated, got {err:?}"
        );
    }

    /// The happy path: an existing, active, linked human resolves — the narrowing removed
    /// the create branch and nothing else.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_only_token_path_resolves_an_existing_active_human(pool: PgPool) {
        let expected = seed_linked_human(&pool, "auth0|known", "known@example.test").await;

        let authed = authenticate_token_existing_only(
            &state(pool.clone()),
            &human_raw("auth0|known", None),
            "tok",
        )
        .await
        .expect("an existing linked human resolves on the lookup-only path");

        assert_eq!(authed.profile.id, expected);
    }

    /// The narrowing must not become a door: a machine token that authenticates fine on the
    /// login path must not reach a profile through the lookup-only path either.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_only_token_path_refuses_a_registered_machine(pool: PgPool) {
        register_machine(&pool, "lookup-only-client").await;

        let err = authenticate_token_existing_only(
            &state(pool.clone()),
            &machine_raw("lookup-only-client"),
            "tok",
        )
        .await
        .expect_err("a machine must not resolve a human profile via the lookup-only path");

        assert!(
            matches!(err, AuthzError::ProfileResolution(ApiError::Unauthorized(ref m)) if m.contains("machine-shaped")),
            "expected a machine-shaped refusal, got {err:?}"
        );
    }

    // Helper: build AuthClaims for a synthetic principal.
    fn claims(sub: &str, email: &str) -> AuthClaims {
        AuthClaims {
            principal_kind: temper_core::types::PrincipalKind::Human,
            provider: "test-provider".to_string(),
            external_user_id: sub.to_string(),
            email: email.to_string(),
            email_verified: Some(true),
            exp: 0,
            iat: 0,
        }
    }

    // Helper: build machine (M2M) AuthClaims for a synthetic agent principal.
    fn machine_claims(client_id: &str) -> AuthClaims {
        AuthClaims {
            principal_kind: temper_core::types::PrincipalKind::Machine,
            provider: MACHINE_PROVIDER_TAG.to_string(),
            external_user_id: client_id.to_string(),
            email: String::new(),
            email_verified: None,
            exp: 0,
            iat: 0,
        }
    }

    /// Register `client_id` against a fresh agent profile. Since G3 Phase A a machine
    /// principal must be registered before it can authenticate at all, so the seam this
    /// module guards is only reachable from the far side of the gate.
    async fn register_machine(pool: &PgPool, client_id: &str) -> uuid::Uuid {
        let profile_id = uuid::Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, $2, $2, NULL, '{}')",
            profile_id,
            format!("agent-{client_id}"),
        )
        .execute(pool)
        .await
        .expect("seed agent profile");
        sqlx::query!(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at) \
             VALUES ($1, $2, $3, $4, NULL, false, true, now())",
            uuid::Uuid::now_v7(),
            profile_id,
            MACHINE_PROVIDER_TAG,
            client_id,
        )
        .execute(pool)
        .await
        .expect("seed agent auth link");
        sqlx::query!(
            "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
             VALUES ($1, 'test', $2, $2)",
            client_id,
            profile_id,
        )
        .execute(pool)
        .await
        .expect("seed registration");
        profile_id
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn registered_machine_principal_rides_ordinary_gate_rails(pool: PgPool) {
        let profile_id = register_machine(&pool, "agent-rails").await;
        // Under D11 a machine is born Denied like every mint door; access is now an APPROVED
        // standing, not an open-mode default. Approve it, and it rides the ordinary gate on the
        // same rail an approved human does.
        crate::test_support::approve(&pool, profile_id).await;

        let c = machine_claims("agent-rails");
        let authed = authenticate(&pool, &c).await.expect("authenticate machine");
        assert!(authed.profile.is_active);
        assert_eq!(
            authed.claims.principal_kind,
            temper_core::types::PrincipalKind::Machine
        );
        require_system_access(&pool, &authed)
            .await
            .expect("an approved machine should be system-authorized, same rail as a human");
    }

    /// The gate is enforced in `temper-services`, so it binds every caller of
    /// `authenticate` — both surfaces — rather than one surface's middleware (D4).
    #[sqlx::test(migrations = "../../migrations")]
    async fn unregistered_machine_principal_never_reaches_the_gate_rails(pool: PgPool) {
        let err = authenticate(&pool, &machine_claims("agent-unknown"))
            .await
            .expect_err("an unregistered machine must not authenticate");
        assert!(
            format!("{err:?}").contains("not registered"),
            "rejection must say why: {err:?}"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn authenticate_returns_active_profile(pool: PgPool) {
        let c = claims("seam-active", "active@example.test");
        let authed = authenticate(&pool, &c).await.expect("should authenticate");
        assert!(authed.profile.is_active);
        assert_eq!(authed.claims.external_user_id, "seam-active");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn authenticate_refuses_deactivated_profile(pool: PgPool) {
        // First resolve creates the profile.
        let c = claims("seam-deactivated", "deact@example.test");
        let authed = authenticate(&pool, &c).await.expect("first resolve");
        let id = authed.profile.id;

        // Soft-delete it (runtime query — test fixture, no macro cache needed).
        sqlx::query("UPDATE kb_profiles SET is_active = false WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("deactivate");

        let err = authenticate(&pool, &c).await.expect_err("should refuse");
        assert!(
            matches!(err, AuthzError::Deactivated { profile_id } if profile_id == id),
            "expected Deactivated, got {err:?}",
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn require_system_access_allows_approved_profile(pool: PgPool) {
        // Under D11 the mint door births a profile Denied, so access is now an APPROVED standing,
        // not an open-mode default. Approve the freshly authenticated profile (legal from Denied,
        // D14), then it is system-authorized.
        let c = claims("seam-approved", "approved@example.test");
        let authed = authenticate(&pool, &c).await.expect("authenticate");
        crate::services::standing_service::apply(
            &pool,
            crate::services::standing_service::ApplyStandingParams {
                subject: ProfileId::from(authed.profile.id),
                act: temper_principal::Act::Approve,
                actor: Some(ProfileId::from(authed.profile.id)),
                authority: temper_principal::ActorAuthority::Admin,
            },
        )
        .await
        .expect("approve");
        let ok = require_system_access(&pool, &authed).await;
        assert!(
            ok.is_ok(),
            "an approved profile should be system-authorized: {ok:?}"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn require_system_access_refuses_when_gated(pool: PgPool) {
        // Enable invite-only so a fresh profile is NOT an approved member.
        // enable_invite_only lives in the e2e harness; here we set the gate
        // directly: point kb_system_settings at a gating team the profile
        // does not belong to.
        let c = claims("seam-gated", "gated@example.test");
        let authed = authenticate(&pool, &c).await.expect("authenticate");
        let id = authed.profile.id;

        sqlx::query(
            "UPDATE kb_system_settings SET access_mode = 'invite_only', \
             gating_team_slug = 'nonexistent-gating-team'",
        )
        .execute(&pool)
        .await
        .expect("enable gate");

        let err = require_system_access(&pool, &authed)
            .await
            .expect_err("gated profile should be refused");
        assert!(
            matches!(err, AuthzError::SystemAccessDenied { profile_id } if profile_id == id),
            "expected SystemAccessDenied, got {err:?}",
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn oauth_first_login_is_born_denied(pool: PgPool) {
        // The community edition has no paywall. An OAuth signup being born Denied — requiring an
        // admin to enable it — IS the access-control mechanism, deliberately (spec §8). Do not
        // "fix" this because new users are locked out. That is the feature.
        let profile = authenticate(&pool, &claims("oauth|newcomer", "a@example.com"))
            .await
            .unwrap();
        let standing =
            crate::services::standing_service::load(&pool, ProfileId::from(profile.profile.id))
                .await
                .unwrap();
        assert_eq!(standing, Some(temper_principal::Standing::Denied));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn saml_jit_is_also_born_denied(pool: PgPool) {
        // D13 — the "assertion IS the grant" rationale was WITHDRAWN by its own author. An IdP
        // asserting "our org says this person may use this" and the instance deciding "we agree,
        // and now they have access" are different claims by different parties, and it is across
        // exactly that boundary that interception and escalation happen. Team assignment already
        // respects this; system access was the odd one out. DO NOT RESTORE AUTO-APPROVAL HERE.
        let profile = resolve_federated_human(
            &pool,
            "test-provider",
            "saml|newcomer",
            "b@example.com",
            Some(true),
        )
        .await
        .unwrap();
        let standing = crate::services::standing_service::load(&pool, ProfileId::from(profile.id))
            .await
            .unwrap();
        assert_eq!(standing, Some(temper_principal::Standing::Denied));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_two_human_doors_mint_the_same_standing(pool: PgPool) {
        // ONE TEST PER PATH, NEVER ONE FOR THE PAIR (§12) — the two doors share
        // create_new_profile_and_link. This third test guards UNIFORMITY where it used to guard
        // divergence. Distinct subjects AND distinct emails: same-email would reconcile the second
        // door onto the FIRST door's profile and the test would pass by resolving one profile twice.
        let oauth = authenticate(&pool, &claims("oauth|d1", "d1@example.com"))
            .await
            .unwrap()
            .profile
            .id;
        let saml = resolve_federated_human(
            &pool,
            "test-provider",
            "saml|d2",
            "d2@example.com",
            Some(true),
        )
        .await
        .unwrap()
        .id;
        assert_ne!(
            oauth, saml,
            "guard: the two doors must have minted two profiles"
        );

        for id in [oauth, saml] {
            assert_eq!(
                crate::services::standing_service::load(&pool, ProfileId::from(id))
                    .await
                    .unwrap(),
                Some(temper_principal::Standing::Denied),
                "both doors must birth Denied — no door grants access (D11)"
            );
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_on_a_returning_principal_does_not_touch_standing(pool: PgPool) {
        // F4, closed structurally. A revoked SAML principal re-asserting through the IdP must stay
        // Revoked — the earlier per-ASSERTION wording made Revoke defeatable on the SAML door.
        let profile = resolve_federated_human(
            &pool,
            "test-provider",
            "saml|returning",
            "r@example.com",
            Some(true),
        )
        .await
        .unwrap();
        let id = ProfileId::from(profile.id);

        // An actor with Admin authority. apply trusts the authority level (the surface is what
        // checks the actor is really an admin, in Beat E); a distinct seeded profile stands in.
        let admin: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('ret-admin','A') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let admin = ProfileId::from(admin);

        use crate::services::standing_service::{apply, ApplyStandingParams};
        use temper_principal::{Act, ActorAuthority};
        apply(
            &pool,
            ApplyStandingParams {
                subject: id,
                act: Act::Approve,
                actor: Some(admin),
                authority: ActorAuthority::Admin,
            },
        )
        .await
        .unwrap();
        apply(
            &pool,
            ApplyStandingParams {
                subject: id,
                act: Act::Revoke {
                    reason: "test".into(),
                },
                actor: Some(admin),
                authority: ActorAuthority::Admin,
            },
        )
        .await
        .unwrap();

        // Re-assert through the IdP.
        resolve_federated_human(
            &pool,
            "test-provider",
            "saml|returning",
            "r@example.com",
            Some(true),
        )
        .await
        .unwrap();

        assert_eq!(
            crate::services::standing_service::load(&pool, id)
                .await
                .unwrap(),
            Some(temper_principal::Standing::Revoked),
            "a returning principal's standing is LOADED, never SET"
        );
    }

    /// Seed a human whose cached auth link stores a real email that the provider
    /// NEVER verified — the ordinary shape for any row created from an unverified
    /// claim, and the shape migration `20260709000004` deliberately backfilled every
    /// pre-existing row into.
    async fn seed_unverified_link(pool: &PgPool, sub: &str, email: &str) -> uuid::Uuid {
        let profile_id = uuid::Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, $2, $2, $3, '{}')",
            profile_id,
            format!("human-{sub}"),
            email,
        )
        .execute(pool)
        .await
        .expect("seed profile");
        sqlx::query!(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at) \
             VALUES ($1, $2, 'test-provider', $3, $4, false, true, now())",
            uuid::Uuid::now_v7(),
            profile_id,
            sub,
            email,
        )
        .execute(pool)
        .await
        .expect("seed unverified link");
        profile_id
    }

    /// An unverified cached email must NOT be promoted to verified by the act of
    /// signing in with a token that carries no email claim.
    ///
    /// FAILS IF: `lookup_cached_email` synthesizes the flag (it returned a hardcoded
    /// `Some(true)` from a query that selected only `email`), or
    /// `resolve_email_from_claims` discards the stored flag and substitutes `Some(true)`
    /// — it did BOTH, and either one alone reopens this.
    ///
    /// The assertion is on the PERSISTED ROW, not on the returned claims, because the
    /// damage is durable: `refresh_link_verification` writes `email_verified = true`,
    /// and that row then satisfies `reconcile_by_email`'s `AND email_verified` filter,
    /// letting a pre-created account holding someone else's address capture that
    /// person's first genuinely-verified sign-in.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unverified_cached_email_is_not_promoted_by_signing_in(pool: PgPool) {
        let profile_id =
            seed_unverified_link(&pool, "auth0|unverified", "victim@example.test").await;

        // A token with NO email claim, so the ladder falls to the cached-link rung.
        let authed = authenticate_token(
            &state(pool.clone()),
            &human_raw("auth0|unverified", None),
            "tok",
        )
        .await
        .expect("a linked human still authenticates from the cached email");
        assert_eq!(authed.profile.id, profile_id);

        let still_unverified = sqlx::query_scalar!(
            "SELECT email_verified FROM kb_profile_auth_links \
             WHERE auth_provider = 'test-provider' AND auth_provider_user_id = $1",
            "auth0|unverified",
        )
        .fetch_one(&pool)
        .await
        .expect("the link row survives the sign-in");

        assert!(
            !still_unverified,
            "signing in with no email claim must not mark an unverified email verified"
        );
    }
}
