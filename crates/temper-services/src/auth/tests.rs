use super::*;
use crate::auth_config::AuthMode;

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
            mcp_audience: "test-audience".to_string(),
            mode: crate::auth_config::AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        cron_host: None,
        sensitivity_sweep_salt: None,
        sensitivity_sweep_enabled: false,
        mcp_service_secret: None,
        vercel_connect: None,
        github_webhook_secret: None,
        slack_link: None,
        slack_mint_secret: None,
        rate_limit: None,
        blob: None,
        blob_disabled_by_policy: false,
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
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'deactivated') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'",
    )
    .bind(id)
    .execute(&pool)
    .await
    .expect("deactivate via standing");

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

/// The narrowing must not become a door: a machine on the lookup-only path takes the machine
/// resolver, which only looks a registration up. A registered machine reaches its own agent
/// profile (the Slack callback then refuses it as a machine, at classification) and an
/// unregistered one is refused; neither creates a profile.
#[sqlx::test(migrations = "../../migrations")]
async fn lookup_only_token_path_resolves_a_machine_without_provisioning(pool: PgPool) {
    let registered = register_machine(&pool, "lookup-only-client").await;
    let count = || async {
        sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .expect("count")
    };
    let before = count().await;

    let authed = authenticate_token_existing_only(
        &state(pool.clone()),
        &machine_raw("lookup-only-client"),
        "tok",
    )
    .await
    .expect("a registered machine resolves to its registration's profile");
    assert_eq!(authed.profile.id, registered);

    let err = authenticate_token_existing_only(
        &state(pool.clone()),
        &machine_raw("lookup-only-stranger"),
        "tok",
    )
    .await
    .expect_err("an unregistered machine is refused, never provisioned");
    assert!(
        !matches!(err, AuthzError::Refused(why) if why == NO_EXISTING_PROFILE),
        "an unregistered machine gets the machine refusal, not the human one: {err:?}"
    );
    assert_eq!(
        count().await,
        before,
        "the lookup-only path created a profile"
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
    let authed = authenticate(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect("authenticate machine");
    // Level 1 passed ⇒ not `Deactivated` standing; the dedicated refuse-tests carry that contract.
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
    let err = authenticate(
        &pool,
        &machine_claims("agent-unknown"),
        AuthMode::ExternalIdp,
    )
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
    let authed = authenticate(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect("should authenticate");
    // Level 1 passed ⇒ not `Deactivated` standing (the is_active proxy was dropped in Phase 2).
    assert_eq!(authed.claims.external_user_id, "seam-active");
}

#[sqlx::test(migrations = "../../migrations")]
async fn authenticate_refuses_deactivated_profile(pool: PgPool) {
    // First resolve creates the profile.
    let c = claims("seam-deactivated", "deact@example.test");
    let authed = authenticate(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect("first resolve");
    let id = authed.profile.id;

    // Deactivate via standing (runtime query — test fixture, no macro cache needed).
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'deactivated') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'",
    )
    .bind(id)
    .execute(&pool)
    .await
    .expect("deactivate via standing");

    let err = authenticate(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect_err("should refuse");
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
    let authed = authenticate(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect("authenticate");
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
    let authed = authenticate(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect("authenticate");
    let id = authed.profile.id;

    // A live gating team the profile is not on: a set gating slug always names one
    // (20261124000050).
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('seam-gating-team', 'gate')")
        .execute(&pool)
        .await
        .expect("the gating team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'seam-gating-team'")
        .execute(&pool)
        .await
        .expect("enable gate");

    let err = require_system_access(&pool, &authed)
        .await
        .expect_err("gated profile should be refused");
    assert!(
        matches!(err, AuthzError::SystemAccessDenied { profile_id, .. } if profile_id == id),
        "expected SystemAccessDenied, got {err:?}",
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn oauth_first_login_is_born_denied(pool: PgPool) {
    // The community edition has no paywall. An OAuth signup being born Denied — requiring an
    // admin to enable it — IS the access-control mechanism, deliberately (spec §8). Do not
    // "fix" this because new users are locked out. That is the feature.
    let profile = authenticate(
        &pool,
        &claims("oauth|newcomer", "a@example.com"),
        AuthMode::ExternalIdp,
    )
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
    let oauth = authenticate(
        &pool,
        &claims("oauth|d1", "d1@example.com"),
        AuthMode::ExternalIdp,
    )
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
    let profile_id = seed_unverified_link(&pool, "auth0|unverified", "victim@example.test").await;

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

// ── Caller classification ───────────────────────────────────────────────────────────────
//
// One witness per arm of `classify_caller`. The claims-only and revoked-row cases are the two
// that isolate a conjunct: the first proves machine claims decide on their own (no row is
// read), the second that the any-row definition — not the unrevoked one — is the one read.

/// Seed a `kb_machine_clients` row for an existing profile, optionally already revoked.
async fn seed_client_row(pool: &PgPool, profile_id: uuid::Uuid, revoked: bool) {
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id, revoked_at) \
         VALUES ($1, 'test', $2, $2, CASE WHEN $3 THEN now() ELSE NULL END)",
        format!("client-{profile_id}"),
        profile_id,
        revoked,
    )
    .execute(pool)
    .await
    .expect("seed client row");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn human_claims_with_no_client_row_classify_human(pool: PgPool) {
    let authed = authenticate(
        &pool,
        &claims("classify-human", "h@example.test"),
        AuthMode::ExternalIdp,
    )
    .await
    .expect("authenticate");

    let caller = classify_caller(&pool, authed).await.expect("classify");

    assert!(matches!(caller, Caller::Human(_)), "got {caller:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn human_claims_with_a_revoked_client_row_classify_machine(pool: PgPool) {
    let authed = authenticate(
        &pool,
        &claims("classify-revoked", "r@example.test"),
        AuthMode::ExternalIdp,
    )
    .await
    .expect("authenticate");
    seed_client_row(&pool, authed.profile.id, true).await;

    let caller = classify_caller(&pool, authed).await.expect("classify");

    assert!(
        matches!(caller, Caller::Machine(_)),
        "a profile that was ever a machine is a machine, revoked or not; got {caller:?}"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn human_claims_with_a_live_client_row_classify_machine_and_log_it(pool: PgPool) {
    let authed = authenticate(
        &pool,
        &claims("classify-mismatch", "m@example.test"),
        AuthMode::ExternalIdp,
    )
    .await
    .expect("authenticate");
    let profile_id = authed.profile.id;
    seed_client_row(&pool, profile_id, false).await;

    let (caller, logs) = crate::test_support::captured_logs(classify_caller(&pool, authed)).await;
    let caller = caller.expect("classify");

    assert!(matches!(caller, Caller::Machine(_)), "got {caller:?}");
    assert!(
        logs.contains("human-shaped token resolved to a machine profile")
            && logs.contains(&profile_id.to_string()),
        "the mismatch must be logged with the profile id; logs: {logs}"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn machine_claims_classify_machine_without_reading_a_row(pool: PgPool) {
    // Machine claims over a profile with NO client row — a state G3 makes unreachable in
    // production — so the claims are the only thing that can make this a machine.
    let authed = authenticate(
        &pool,
        &claims("classify-claims", "c@example.test"),
        AuthMode::ExternalIdp,
    )
    .await
    .expect("authenticate");
    let machine =
        gate_resolved_profile(&pool, authed.into_profile(), &machine_claims("claims-only"))
            .await
            .expect("gate");

    let caller = classify_caller(&pool, machine).await.expect("classify");

    assert!(matches!(caller, Caller::Machine(_)), "got {caller:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_failed_machine_read_refuses_as_internal_never_human(pool: PgPool) {
    let authed = authenticate(
        &pool,
        &claims("classify-fail", "f@example.test"),
        AuthMode::ExternalIdp,
    )
    .await
    .expect("authenticate");
    pool.close().await;

    let err = classify_caller(&pool, authed)
        .await
        .expect_err("a failed read must refuse, not fall through to Human");

    assert!(
        matches!(err, AuthzError::Classification(ApiError::Internal(_))),
        "got {err:?}"
    );
}
