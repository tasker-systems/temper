use super::*;
use sqlx::PgPool;

#[test]
fn oversized_preferences_rejected() {
    let large_value: serde_json::Value = serde_json::Value::String("x".repeat(65_537));
    let result = validate_preferences_size(Some(&large_value));
    assert!(result.is_err());
}

#[test]
fn normal_preferences_accepted() {
    let small_value: serde_json::Value = serde_json::json!({"theme": "dark"});
    let result = validate_preferences_size(Some(&small_value));
    assert!(result.is_ok());
}

#[test]
fn none_preferences_accepted() {
    let result = validate_preferences_size(None);
    assert!(result.is_ok());
}

#[sqlx::test(migrations = "../../migrations")]
async fn generate_handle_from_display_name(pool: PgPool) {
    let handle = generate_profile_handle(&pool, "Pete Taylor").await.unwrap();
    assert_eq!(handle, "pete-taylor");
}

#[sqlx::test(migrations = "../../migrations")]
async fn generate_handle_handles_special_chars(pool: PgPool) {
    let handle = generate_profile_handle(&pool, "José García-López")
        .await
        .unwrap();
    assert_eq!(handle, "jos-garc-a-l-pez");
}

#[sqlx::test(migrations = "../../migrations")]
async fn generate_handle_handles_collision(pool: PgPool) {
    // Create a profile that will own the "collider" handle
    let claims = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "test".to_string(),
        external_user_id: "slug-collision-1".to_string(),
        email: "collider@example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile = resolve_from_claims(&pool, &claims, AuthMode::ExternalIdp)
        .await
        .unwrap();
    assert_eq!(profile.slug, "collider");

    // Now generate a handle for the same display name — should get -2
    let handle = generate_profile_handle(&pool, "collider").await.unwrap();
    assert_eq!(handle, "collider-2");
}

#[sqlx::test(migrations = "../../migrations")]
async fn verified_email_reconciles_to_existing_profile(pool: PgPool) {
    let claims_a = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_a".to_string(),
        external_user_id: "user-recon-verified-a".to_string(),
        email: "recon-verified@example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_a = resolve_from_claims(&pool, &claims_a, AuthMode::ExternalIdp)
        .await
        .unwrap();

    let claims_b = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_b".to_string(),
        external_user_id: "user-recon-verified-b".to_string(),
        email: "recon-verified@example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_b = resolve_from_claims(&pool, &claims_b, AuthMode::ExternalIdp)
        .await
        .unwrap();

    assert_eq!(
        profile_a.id, profile_b.id,
        "verified email should reconcile to same profile"
    );
}

/// Providers that disagree on case are one person: a verified sign-in whose address differs
/// from an existing verified link only in case joins that profile, rather than minting a
/// second one that would leave the address addressing nobody.
#[sqlx::test(migrations = "../../migrations")]
async fn verified_email_reconciles_across_case(pool: PgPool) {
    let claims_a = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_a".to_string(),
        external_user_id: "user-recon-case-a".to_string(),
        email: "Recon-Case@Example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_a = resolve_from_claims(&pool, &claims_a, AuthMode::ExternalIdp)
        .await
        .unwrap();

    let claims_b = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_b".to_string(),
        external_user_id: "user-recon-case-b".to_string(),
        email: "recon-case@example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_b = resolve_from_claims(&pool, &claims_b, AuthMode::ExternalIdp)
        .await
        .unwrap();

    assert_eq!(
        profile_a.id, profile_b.id,
        "a case variant of a verified address is the same person"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn unverified_email_creates_separate_profile(pool: PgPool) {
    let claims_a = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_a".to_string(),
        external_user_id: "user-recon-unverified-a".to_string(),
        email: "recon-unverified@example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_a = resolve_from_claims(&pool, &claims_a, AuthMode::ExternalIdp)
        .await
        .unwrap();

    let claims_b = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_b".to_string(),
        external_user_id: "user-recon-unverified-b".to_string(),
        email: "recon-unverified@example.com".to_string(),
        email_verified: Some(false),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_b = resolve_from_claims(&pool, &claims_b, AuthMode::ExternalIdp)
        .await
        .unwrap();

    assert_ne!(
        profile_a.id, profile_b.id,
        "unverified email should create separate profile"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn missing_email_verified_creates_separate_profile(pool: PgPool) {
    let claims_a = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_a".to_string(),
        external_user_id: "user-recon-none-a".to_string(),
        email: "recon-none@example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_a = resolve_from_claims(&pool, &claims_a, AuthMode::ExternalIdp)
        .await
        .unwrap();

    let claims_b = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_b".to_string(),
        external_user_id: "user-recon-none-b".to_string(),
        email: "recon-none@example.com".to_string(),
        email_verified: None,
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_b = resolve_from_claims(&pool, &claims_b, AuthMode::ExternalIdp)
        .await
        .unwrap();

    assert_ne!(
        profile_a.id, profile_b.id,
        "None email_verified should create separate profile"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn unverified_stored_link_does_not_capture_verified_signin(pool: PgPool) {
    // Profile A signed up with an UNVERIFIED email — its stored link is
    // unverified. A later VERIFIED sign-in with the same email (e.g. the
    // address's real owner) must NOT reconcile onto A's profile: an
    // unverified stored email is an attacker-controllable claim.
    let claims_a = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_a".to_string(),
        external_user_id: "user-stored-unverified-a".to_string(),
        email: "stored-unverified@example.com".to_string(),
        email_verified: Some(false),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_a = resolve_from_claims(&pool, &claims_a, AuthMode::ExternalIdp)
        .await
        .unwrap();

    let claims_b = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_b".to_string(),
        external_user_id: "user-stored-unverified-b".to_string(),
        email: "stored-unverified@example.com".to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile_b = resolve_from_claims(&pool, &claims_b, AuthMode::ExternalIdp)
        .await
        .unwrap();

    assert_ne!(
        profile_a.id, profile_b.id,
        "a verified sign-in must not attach to a profile whose stored email is unverified"
    );
}

fn human(provider: &str, uid: &str, email: &str, verified: bool) -> AuthClaims {
    AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: provider.to_string(),
        external_user_id: uid.to_string(),
        email: email.to_string(),
        email_verified: Some(verified),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    }
}

/// The case-blind match must not widen the unverified-capture guard: a stored UNVERIFIED link
/// at a case variant captures no verified sign-in.
#[sqlx::test(migrations = "../../migrations")]
async fn unverified_case_variant_link_does_not_capture_verified_signin(pool: PgPool) {
    let a = resolve_from_claims(
        &pool,
        &human("provider_a", "cv-a", "Stored-CV@example.com", false),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();
    let b = resolve_from_claims(
        &pool,
        &human("provider_b", "cv-b", "stored-cv@example.com", true),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();
    assert_ne!(a.id, b.id);
}

/// A look-alike is a different address: U+212A KELVIN SIGN, which the database collation's
/// `lower()` folds to `k`, must not link to the `k` address.
#[sqlx::test(migrations = "../../migrations")]
async fn a_look_alike_address_does_not_link(pool: PgPool) {
    // Precondition: the test database's own fold maps the Kelvin sign to `k`, so this
    // test fails if the ASCII-only fold is dropped. Under a C-locale database it would not.
    let db_folds: bool = sqlx::query_scalar("SELECT lower(E'\\u212A') = 'k'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        db_folds,
        "test database no longer folds U+212A; this witness has no bite"
    );
    let a = resolve_from_claims(
        &pool,
        &human("provider_a", "la-a", "kate@example.com", true),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();
    let b = resolve_from_claims(
        &pool,
        &human("provider_b", "la-b", "\u{212A}ate@example.com", true),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();
    assert_ne!(a.id, b.id, "a look-alike address is not the same address");
}

/// Legacy duplicates: two profiles already hold case variants of one verified address. A
/// sign-in links to the exact-case holder when there is one, else to the oldest.
#[sqlx::test(migrations = "../../migrations")]
async fn a_shared_address_links_to_the_exact_case_then_the_oldest_holder(pool: PgPool) {
    let older = resolve_from_claims(
        &pool,
        &human("provider_a", "dup-a", "Dup@example.com", true),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();
    // A second profile on a case variant, as legacy data could hold it: inserted directly,
    // since sign-in no longer creates one.
    let newer_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, 'dup-newer', 'n')")
        .bind(newer_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO kb_profile_auth_links \
           (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at) \
         VALUES ($1, $2, 'provider_b', 'dup-b', 'dup@example.com', true, true, now() + interval '1 minute')",
    )
    .bind(Uuid::now_v7())
    .bind(newer_id)
    .execute(&pool)
    .await
    .unwrap();

    let exact = resolve_from_claims(
        &pool,
        &human("provider_c", "dup-c", "dup@example.com", true),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();
    assert_eq!(exact.id, newer_id, "the exact-case holder wins");
    let other = resolve_from_claims(
        &pool,
        &human("provider_d", "dup-d", "DUP@example.com", true),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();
    assert_eq!(
        other.id, older.id,
        "with no exact-case holder, the oldest wins"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn verified_signin_refreshes_stored_link(pool: PgPool) {
    // First sign-in unverified → link stored unverified. Second sign-in on
    // the SAME provider identity, now verified → the stored flag self-heals.
    let mut claims = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "provider_a".to_string(),
        external_user_id: "user-refresh".to_string(),
        email: "refresh@example.com".to_string(),
        email_verified: Some(false),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    };
    let profile = resolve_from_claims(&pool, &claims, AuthMode::ExternalIdp)
        .await
        .unwrap();

    let stored: bool = sqlx::query_scalar(
        "SELECT email_verified FROM kb_profile_auth_links \
         WHERE auth_provider = 'provider_a' AND auth_provider_user_id = 'user-refresh'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!stored, "first (unverified) sign-in stores unverified");

    claims.email_verified = Some(true);
    let same = resolve_from_claims(&pool, &claims, AuthMode::ExternalIdp)
        .await
        .unwrap();
    assert_eq!(profile.id, same.id);

    let stored: bool = sqlx::query_scalar(
        "SELECT email_verified FROM kb_profile_auth_links \
         WHERE auth_provider = 'provider_a' AND auth_provider_user_id = 'user-refresh'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(stored, "verified sign-in refreshes the stored flag");
}

/// A plain verified human principal — the shape the surrounding tests inline.
fn human_claims(sub: &str, email: &str) -> AuthClaims {
    AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "test-provider".to_string(),
        external_user_id: sub.to_string(),
        email: email.to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    }
}

/// The D3 invariant. Asserting the None alone would not catch a regression that
/// creates the profile and THEN errors, so assert the absence of the row too.
#[sqlx::test(migrations = "../../migrations")]
async fn lookup_only_refuses_an_unknown_sub_and_creates_no_profile(pool: PgPool) {
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .unwrap();

    let claims = human_claims("auth0|never-seen-before", "nobody@example.com");
    let resolved = resolve_existing_human_from_claims(&pool, &claims)
        .await
        .unwrap();
    assert!(resolved.is_none(), "an unknown sub must not resolve");

    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after, "lookup-only must not mint a profile");
}

#[sqlx::test(migrations = "../../migrations")]
async fn lookup_only_resolves_an_existing_linked_profile(pool: PgPool) {
    let claims = human_claims("auth0|existing", "someone@example.com");
    // The normal login path mints it once...
    let created = resolve_from_claims(&pool, &claims, AuthMode::ExternalIdp)
        .await
        .unwrap();

    // ...and lookup-only then finds that same profile.
    let found = resolve_existing_human_from_claims(&pool, &claims)
        .await
        .unwrap()
        .expect("an existing link must resolve");
    assert_eq!(found.id, created.id);
}

/// Step 3-4 are KEPT: a verified email attaches to an EXISTING profile. This is the
/// probe that the narrowing removed *only* the create branch — if reconcile were
/// refused too, this resolves to None instead of the sibling profile.
#[sqlx::test(migrations = "../../migrations")]
async fn lookup_only_reconciles_a_verified_email_to_an_existing_profile(pool: PgPool) {
    let existing = resolve_from_claims(
        &pool,
        &human_claims("auth0|first", "shared@example.com"),
        AuthMode::ExternalIdp,
    )
    .await
    .unwrap();

    let mut other_provider = human_claims("auth0|second", "shared@example.com");
    other_provider.provider = "other-provider".to_string();

    let found = resolve_existing_human_from_claims(&pool, &other_provider)
        .await
        .unwrap()
        .expect("a verified email must reconcile onto the existing profile");
    assert_eq!(found.id, existing.id);
}

/// A machine-shaped identity is refused here as it is everywhere else.
#[sqlx::test(migrations = "../../migrations")]
async fn lookup_only_refuses_a_machine_shaped_identity(pool: PgPool) {
    let claims = machine_shaped_human_claims("abc123@clients", "auth0");
    assert!(resolve_existing_human_from_claims(&pool, &claims)
        .await
        .is_err());
}

fn machine_claims(client_id: &str) -> AuthClaims {
    AuthClaims {
        principal_kind: PrincipalKind::Machine,
        provider: crate::auth::MACHINE_PROVIDER_TAG.to_string(),
        external_user_id: client_id.to_string(),
        email: String::new(),
        email_verified: None,
        exp: 0,
        iat: 0,
    }
}

/// The bite test. Under the old code this FAILS by finding a newly created profile.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn unregistered_machine_is_rejected_and_creates_no_profile(pool: PgPool) {
    let before = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count before");

    let c = machine_claims("never-registered");
    let err = resolve_from_claims(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect_err("an unregistered machine must be rejected");

    match err {
        ApiError::Unauthorized(msg) => {
            assert!(
                msg.contains("never-registered"),
                "message names the client id: {msg}"
            );
            assert!(msg.contains("not registered"), "message says why: {msg}");
            assert!(
                msg.contains("temper admin machine provision"),
                "message names the remedy: {msg}"
            );
        }
        other => panic!("expected Unauthorized, got {other:?}"),
    }

    let after = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count after");
    assert_eq!(
        before, after,
        "authentication must not create a profile (D3)"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoked_machine_is_rejected_distinguishably(pool: PgPool) {
    // Seed a profile + registration, then revoke it.
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-revoked', 'agent-revoked', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id, revoked_at) \
         VALUES ('dead-client', 'test', $1, $1, now())",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed revoked client");

    let err = resolve_from_claims(&pool, &machine_claims("dead-client"), AuthMode::ExternalIdp)
        .await
        .expect_err("a revoked machine must be rejected");

    match err {
        ApiError::Unauthorized(msg) => {
            assert!(
                msg.contains("dead-client"),
                "message names the client id: {msg}"
            );
            assert!(
                msg.contains("revoked"),
                "revoked must be distinguishable from unregistered (D7): {msg}"
            );
        }
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

/// A client registered through dynamic client registration can obtain a client_credentials
/// token from temper's own authorization server (its row lives in `kb_oauth_dcr_clients`,
/// seeded here as production holds it), but it has no `kb_machine_clients` row, so the machine
/// gate refuses it, in the minted-namespace wording, before any profile exists.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_dynamically_registered_client_is_refused_and_creates_no_profile(pool: PgPool) {
    let client_id = "conn_fence0123456789";
    sqlx::query(
        "INSERT INTO kb_oauth_dcr_clients \
           (client_id, client_secret_hash, grant_types, redirect_uris, token_endpoint_auth_method) \
         VALUES ($1, 'x', ARRAY['client_credentials'], ARRAY['https://connect.vercel.com/callback'], \
                 'client_secret_basic')",
    )
    .bind(client_id)
    .execute(&pool)
    .await
    .expect("seed dcr client");
    let profiles = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .expect("count")
    };
    let before = profiles().await;

    for mode in [AuthMode::TemperAs, AuthMode::ExternalIdp] {
        let err = resolve_from_claims(&pool, &machine_claims(client_id), mode)
            .await
            .expect_err("a DCR client is not a machine principal");
        match err {
            ApiError::Unauthorized(msg) => assert!(
                msg.contains("not registered")
                    && msg.contains(client_id)
                    && msg.contains("namespace temper mints"),
                "refused as unregistered in a minted namespace, naming the client: {msg}"
            ),
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    assert_eq!(before, profiles().await, "no profile is created");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn registered_machine_resolves_to_its_profile(pool: PgPool) {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-live', 'agent-live', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ('live-client', 'test', $1, $1)",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed client");

    let profile = resolve_from_claims(&pool, &machine_claims("live-client"), AuthMode::ExternalIdp)
        .await
        .expect("registered machine resolves");
    assert_eq!(profile.id, profile_id);

    // The gate touched last_seen_at.
    let seen = sqlx::query_scalar!(
        "SELECT last_seen_at FROM kb_machine_clients WHERE client_id = 'live-client'"
    )
    .fetch_one(&pool)
    .await
    .expect("read last_seen");
    assert!(seen.is_some(), "the gate records liveness");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn machine_resolution_is_idempotent(pool: PgPool) {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-idem', 'agent-idem', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ('agent-idem', 'test', $1, $1)",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed client");

    let c = machine_claims("agent-idem");
    let first = resolve_from_claims(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect("first");
    let second = resolve_from_claims(&pool, &c, AuthMode::ExternalIdp)
        .await
        .expect("second");
    assert_eq!(first.id, second.id, "resolution is stable across calls");
}

/// Seed a live machine registration for `client_id` under `issuer`.
async fn seed_machine_under(pool: &PgPool, client_id: &str, issuer: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
        profile_id,
        format!("agent-{client_id}"),
    )
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, issuer, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $2, 'test', $3, $3)",
        client_id,
        issuer,
        profile_id,
    )
    .execute(pool)
    .await
    .expect("seed client");
    profile_id
}

/// A token resolves only to a registration its instance's issuer could have minted. Each
/// deployment trusts one issuer, so a token naming a client id registered under the other one
/// is a capture, and is refused with the way back named.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_machine_registered_under_the_other_issuer_is_refused(pool: PgPool) {
    let external = seed_machine_under(&pool, "idp-client", "auth0-m2m").await;
    let minted = seed_machine_under(&pool, "temper-client", "temper").await;

    // Neither resolves under the other issuer's mode.
    match resolve_from_claims(&pool, &machine_claims("idp-client"), AuthMode::TemperAs).await {
        Err(ApiError::Unauthorized(msg)) => {
            assert!(msg.contains("idp-client"), "names the client: {msg}");
            assert!(
                msg.contains("temper admin machine issue"),
                "names the way back: {msg}"
            );
        }
        other => panic!("an external registration must not resolve under an AS: {other:?}"),
    }
    match resolve_from_claims(
        &pool,
        &machine_claims("temper-client"),
        AuthMode::ExternalIdp,
    )
    .await
    {
        Err(ApiError::Unauthorized(msg)) => {
            assert!(msg.contains("temper-client"), "names the client: {msg}");
            assert!(msg.contains("temper-issued"), "says why: {msg}");
        }
        other => {
            panic!("a temper registration must not resolve under an external IdP: {other:?}")
        }
    }

    // A refused resolution records no liveness: the gate refuses before it touches the row.
    let touched = sqlx::query_scalar!(
        "SELECT count(*) AS \"n!\" FROM kb_machine_clients WHERE last_seen_at IS NOT NULL"
    )
    .fetch_one(&pool)
    .await
    .expect("read last_seen");
    assert_eq!(touched, 0, "a refused machine is not recorded as seen");

    // Each resolves under the mode whose issuer minted it.
    let p = resolve_from_claims(&pool, &machine_claims("idp-client"), AuthMode::ExternalIdp)
        .await
        .expect("an external registration resolves under an external IdP");
    assert_eq!(p.id, external);
    let p = resolve_from_claims(&pool, &machine_claims("temper-client"), AuthMode::TemperAs)
        .await
        .expect("a temper registration resolves under temper's own AS");
    assert_eq!(p.id, minted);
}

/// Claims that *say* Human but carry a machine-shaped identity — the exact shape a
/// classification fall-open produces. `sub` ends in the Auth0 `@clients` suffix but the
/// token never declared `gty`, so a surface that treats "not recognized as machine" as
/// "therefore human" hands these to the human path.
fn machine_shaped_human_claims(external_user_id: &str, provider: &str) -> AuthClaims {
    AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: provider.to_string(),
        external_user_id: external_user_id.to_string(),
        // An M2M token has no email; temper-mcp fills `String::new()` unconditionally.
        email: String::new(),
        email_verified: None,
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    }
}

/// The bite test for the human path. Before the Layer-2 guard this FAILS by
/// auto-provisioning a human profile for a `@clients` identity — the
/// `kb_machine_clients` registration gate bypassed entirely, because the gate only
/// guards `PrincipalKind::Machine`.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn human_path_refuses_clients_suffixed_identity(pool: PgPool) {
    let before = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count before");

    let err = resolve_from_claims(
        &pool,
        &machine_shaped_human_claims("abc123@clients", "auth0"),
        AuthMode::ExternalIdp,
    )
    .await
    .expect_err("a machine-shaped identity must never resolve to a human profile");

    match err {
        ApiError::Unauthorized(msg) => {
            assert!(
                msg.contains("machine"),
                "the refusal must name the reason: {msg}"
            );
        }
        other => panic!("expected Unauthorized, got {other:?}"),
    }

    let after = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count after");
    assert_eq!(
        before, after,
        "the human path must not auto-provision a profile for a machine-shaped identity"
    );
}

/// Same guard, keyed on the provider tag rather than the `sub` shape: a caller that
/// mislabels machine-provider claims as Human must not reach the create branch either.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn human_path_refuses_machine_provider_tag(pool: PgPool) {
    let err = resolve_from_claims(
        &pool,
        &machine_shaped_human_claims("plain-client-id", crate::auth::MACHINE_PROVIDER_TAG),
        AuthMode::ExternalIdp,
    )
    .await
    .expect_err("machine-provider claims must never resolve to a human profile");

    assert!(
        matches!(err, ApiError::Unauthorized(ref msg) if msg.contains("machine")),
        "expected a machine-shaped Unauthorized, got {err:?}"
    );
}

async fn seed_bare_profile(pool: &PgPool, handle: &str) -> Uuid {
    crate::test_support::profile::insert(pool, handle).await
}

async fn emitter_count(pool: &PgPool, profile_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_entities WHERE profile_id = $1")
        .bind(profile_id)
        .fetch_one(pool)
        .await
        .expect("count emitters")
}

/// Two first-authenticated requests for the same new profile can run concurrently. Each
/// emitter is its own auto-commit statement, so before the unique constraint on
/// `(profile_id, name)` both writers observed no row and both inserted — silently splitting
/// one logical emitter across two `entity_id`s.
#[sqlx::test(migrations = "../../migrations")]
async fn provision_profile_entities_is_safe_under_concurrent_invocation(pool: PgPool) {
    let handle = "concurrent-provision";
    let profile_id = seed_bare_profile(&pool, handle).await;

    let mut conn_a = pool.acquire().await.expect("acquire conn a");
    let mut conn_b = pool.acquire().await.expect("acquire conn b");
    let (a, b) = tokio::join!(
        provision_profile_entities(&mut conn_a, profile_id, handle),
        provision_profile_entities(&mut conn_b, profile_id, handle),
    );
    a.expect("first concurrent provision");
    b.expect("second concurrent provision");

    assert_eq!(
        emitter_count(&pool, profile_id).await,
        Surface::ALL.len() as i64,
        "concurrent provisioning must yield exactly one emitter per surface",
    );
}

async fn count_by_profile(pool: &PgPool, table: &str, profile_id: Uuid) -> i64 {
    // The column is `id` on kb_profiles and `profile_id` everywhere else.
    let column = if table == "kb_profiles" {
        "id"
    } else {
        "profile_id"
    };
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE {column} = $1"))
        .bind(profile_id)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("count {table}: {e}"))
}

/// The stranding bite test. Step 5 of `resolve_human_from_claims` used to be two
/// auto-committing statements on separate connections, so a failure *after* the profile
/// committed left it behind with zero `kb_entities` — and step 1's link lookup then
/// short-circuited every retry, so provisioning never ran again and the profile was stranded
/// for good.
///
/// Rolling the transaction back stands in for "the request died after the profile was
/// created". What makes this a guard rather than a tautology is the STANDING assertion: the
/// realistic regression is someone reaching for the pool-taking `standing_service::provision`
/// (or `generate_profile_handle`, or a bare `.execute(pool)`) inside this transaction. Such a
/// write lands on a different connection and SURVIVES this rollback, so the standing row would
/// outlive the profile it describes and this test would go red naming it.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_interrupted_registration_leaves_no_stranded_profile(pool: PgPool) {
    let claims = human_claims("auth0|interrupted-registration", "stranded@example.com");

    let mut tx = pool.begin().await.expect("begin");
    let (profile_id, handle) = create_new_profile_and_link(&mut tx, &claims)
        .await
        .expect("create profile and link");
    provision_profile_entities(&mut tx, profile_id, &handle)
        .await
        .expect("provision entities");

    // The interruption: real work, and then no commit.
    tx.rollback().await.expect("rollback");

    for table in [
        "kb_profiles",
        "kb_profile_auth_links",
        "kb_entities",
        "kb_principal_standing",
    ] {
        assert_eq!(
            count_by_profile(&pool, table, profile_id).await,
            0,
            "an interrupted registration must leave no {table} row — a profile that survives \
             without its emitters can never be healed, because the auth-link lookup makes \
             every retry return early",
        );
    }
}

/// The other half of the acceptance: the already-working path is unchanged. A first sign-in
/// still lands a fully provisioned profile — every surface emitter, plus the D11 standing row
/// every mint door births — now committed together rather than in sequence.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_first_sign_in_commits_profile_entities_and_standing_together(pool: PgPool) {
    let claims = human_claims("auth0|first-sign-in", "newcomer@example.com");

    let profile = resolve_from_claims(&pool, &claims, AuthMode::ExternalIdp)
        .await
        .expect("first sign-in resolves a profile");

    assert_eq!(
        emitter_count(&pool, profile.id).await,
        Surface::ALL.len() as i64,
        "a first sign-in must provision one emitter per surface",
    );
    assert_eq!(
        count_by_profile(&pool, "kb_principal_standing", profile.id).await,
        1,
        "D11 — every mint door births standing",
    );

    // Signing in again is the early-return path (step 1), which must create nothing further.
    let again = resolve_from_claims(&pool, &claims, AuthMode::ExternalIdp)
        .await
        .expect("second sign-in resolves the same profile");
    assert_eq!(
        again.id, profile.id,
        "the second sign-in must not mint anew"
    );
    assert_eq!(
        emitter_count(&pool, profile.id).await,
        Surface::ALL.len() as i64,
    );
}

/// The sequential case the constraint also unlocks: provisioning is now safe to re-run, so a
/// profile left half-provisioned by a failed request can be repaired by calling it again.
#[sqlx::test(migrations = "../../migrations")]
async fn provision_profile_entities_is_idempotent(pool: PgPool) {
    let handle = "repeat-provision";
    let profile_id = seed_bare_profile(&pool, handle).await;

    let mut conn = pool.acquire().await.expect("acquire conn");
    provision_profile_entities(&mut conn, profile_id, handle)
        .await
        .expect("first provision");
    provision_profile_entities(&mut conn, profile_id, handle)
        .await
        .expect("second provision");

    assert_eq!(
        emitter_count(&pool, profile_id).await,
        Surface::ALL.len() as i64,
    );
}
