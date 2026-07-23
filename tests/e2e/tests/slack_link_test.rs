#![cfg(feature = "test-db")]
//! The Slack account-link flow, end to end: intent → IdP → callback → link row.
//!
//! `test-db` green is a false signal for access-semantics changes, and this is squarely
//! one — the callback authenticates a human against a token minted by an external IdP and
//! decides whether a profile exists. That decision only exists once the HMAC gate, the
//! intent burn, the token exchange and the JWKS verification are all in the same process.
//! Hence this tier.
//!
//! Two things this file does that the sibling tests do not, both forced by the flow:
//!
//! 1. **The issuer is a wiremock server, not `"test-issuer"`.** `link_provider::derive`
//!    builds the token endpoint from `AuthConfig.issuer`, so pointing the issuer anywhere
//!    else would leave the exchange with nothing to talk to. Everything downstream follows:
//!    the access token the stub returns must carry that same `iss`, because the callback
//!    re-verifies it through the real JWKS path.
//! 2. **The access token is a real JWT signed with the RSA fixture.** `resolve_existing`
//!    runs `jsonwebtoken::decode` against the key store before it resolves anything, so an
//!    opaque placeholder would fail *verification* and never reach the lookup — the tests
//!    would pass for the wrong reason and prove nothing about lookup-only.

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use sqlx::PgPool;
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use tempfile::TempDir;

use temper_api::create_app;
use temper_core::types::config::{CloudSection, CloudVaultConfig, TemperConfig};
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::{ApiConfig, SlackLinkConfig};
use temper_services::state::{AppState, JwksKeyStore};

/// The WHOLE opaque principal. Four segments' worth of Slack ids joined by colons —
/// carried verbatim, never split (spec: the segment count is not ours to assume).
const SLACK_PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

/// Shared with the "mention agent" (this test, standing in for it). Gates the intent route.
const SLACK_SECRET: &str = "slack-link-e2e-secret";

/// The mint gate's key. **Deliberately a different value from [`SLACK_SECRET`]**, because the
/// separation of these two keys is itself the security property under test: link-state answers a
/// question, the mint endpoint vends an act-as-the-human token, and a compromise of the former
/// must not confer the latter. `mint_refuses_the_link_state_key` is the bite probe — it signs a
/// mint call with `SLACK_SECRET` and requires a 401, so collapsing these into one variable turns
/// that test red instead of silently widening the gate.
const MINT_SECRET: &str = "slack-mint-e2e-secret";

const CLIENT_ID: &str = "slack-link-client";

/// A fixed 32-byte vault key (base64), so the grant vault can seal the RT the stub hands back.
/// `AQEB…` decodes to 32 bytes of `0x01` — enough for XChaCha20-Poly1305; the value is
/// irrelevant, only that it parses.
const VAULT_KEY_B64: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

/// The refresh token the stub IdP returns on every exchange (see [`stub_token_endpoint`]).
const STUB_REFRESH_TOKEN: &str = "stub-refresh-token";

fn vault_key() -> temper_services::services::grant_crypto::VaultKey {
    temper_services::services::grant_crypto::VaultKey::from_base64(VAULT_KEY_B64)
        .expect("the fixed test vault key parses")
}

/// Matches `ApiConfig.auth_provider_name` below. The lookup-only resolve keys on
/// `(auth_provider, external_user_id)` using the SERVER's configured provider name, so an
/// "existing profile" means a `kb_profile_auth_links` row under exactly this string.
const PROVIDER: &str = "test-provider";

/// The authorization code the stub IdP accepts. Opaque to everything under test.
const AUTH_CODE: &str = "test-authorization-code";

/// The bearer secret gating the intents-reaper cron.
///
/// Set on this harness (the sibling e2e harnesses leave `embed_dispatch_secret: None`) so the
/// reaper test can assert BOTH arms. With the secret unset, `require_dispatch_secret` refuses
/// every caller — a "no bearer → 401" test would then pass against a handler with the gate
/// deleted, because the endpoint would be off for an unrelated reason.
const REAP_SECRET: &str = "slack-reap-e2e-secret";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A running API server whose configured issuer IS `idp` — see the module doc.
struct SlackLinkApp {
    addr: std::net::SocketAddr,
    http: reqwest::Client,
    /// Kept alive for the test's duration: dropping it stops serving the token endpoint.
    idp: MockServer,
    /// Backs `cli_config_path()`. Kept alive for the test's duration so the materialized
    /// config file survives until the spawned CLI has read it; dropping it deletes the dir.
    cli_config_dir: TempDir,
}

impl SlackLinkApp {
    fn issuer(&self) -> String {
        self.idp.uri()
    }

    fn callback_url(&self, state_nonce: &str) -> String {
        format!(
            "http://{}/api/auth/slack/callback?code={AUTH_CODE}&state={state_nonce}",
            self.addr
        )
    }

    /// Materialize a minimal global CLI config pointed at this app's server, mirroring what
    /// `common::run_temper_cli` writes for `E2eTestApp`. `SlackLinkApp` has no `TemperConfig`
    /// of its own (it never builds a `temper_client::TemperClient`), so this builds one just
    /// for the spawned-binary CLI's `TEMPER_GLOBAL_CONFIG` to read. The vault path itself is
    /// never touched by a `slack disconnect` invocation, but `CloudVaultConfig` validates
    /// non-empty, so it must point somewhere real.
    fn cli_config_path(&self) -> std::path::PathBuf {
        let config = TemperConfig {
            vault: CloudVaultConfig {
                path: self.cli_config_dir.path().to_str().unwrap().to_string(),
            },
            cloud: CloudSection {
                api_url: format!("http://{}", self.addr),
            },
            ..TemperConfig::default()
        };
        let config_toml = toml::to_string(&config).expect("serialize SlackLinkApp CLI config");
        let config_path = self.cli_config_dir.path().join("test-temper-config.toml");
        std::fs::write(&config_path, config_toml).expect("write SlackLinkApp CLI config");
        config_path
    }
}

/// Spawn the API with the Slack link fully configured and the issuer pointed at a stub IdP.
///
/// Mirrors `common::setup`'s server half (same RSA fixture, same static key store, same
/// `create_app`); it does not reuse it because `setup` hard-codes `issuer: "test-issuer"`
/// and `slack_link: None`, which are the two values this flow turns on.
async fn setup_slack_app(pool: &PgPool) -> SlackLinkApp {
    setup_slack_app_with_mint_secret(pool, Some(MINT_SECRET)).await
}

/// As [`setup_slack_app`], but with the mint gate's key injectable so a test can spawn an
/// instance with minting **unconfigured**.
///
/// `SLACK_MINT_SECRET` is deliberately independent of `SlackLinkConfig`'s all-or-nothing set, so
/// "link flow on, minting off" is a real and reachable deployment state — not a misconfiguration.
/// It is what every instance running today looks like before the mint secret is set, which is
/// precisely why it needs a test rather than an assumption.
async fn setup_slack_app_with_mint_secret(
    pool: &PgPool,
    mint_secret: Option<&str>,
) -> SlackLinkApp {
    let idp = MockServer::start().await;

    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);

    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            // The stub IdP. `link_provider::derive` turns this into `{issuer}/oauth/token`
            // under `ExternalIdp`, which is the route the stub below mounts.
            issuer: idp.uri(),
            jwks_url: "unused".to_string(),
            audience: common::TEST_AUDIENCE.to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: PROVIDER.to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: Some(REAP_SECRET.to_string()),
        vercel_connect: None,
        slack_link: Some(SlackLinkConfig {
            client_id: CLIENT_ID.to_string(),
            hmac_secret: SLACK_SECRET.to_string(),
            public_base_url: "https://temper.test".to_string(),
            vault_key: vault_key(),
        }),
        slack_mint_secret: mint_secret.map(str::to_owned),
    };

    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test server");
    });

    SlackLinkApp {
        addr,
        http: reqwest::Client::new(),
        idp,
        cli_config_dir: TempDir::new().expect("temp dir for SlackLinkApp CLI config"),
    }
}

#[derive(Debug, Serialize)]
struct LinkJwtClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// Sign an access token the way the stub IdP would: same RSA fixture the server verifies
/// against, `iss` set to the stub's own URI, `aud` the instance's audience.
///
/// `common::generate_test_jwt` cannot serve here — it hard-codes `iss: "test-issuer"`, and
/// this instance's issuer is the stub. The `email` claim is present deliberately: the auth
/// seam's email ladder would otherwise fall through to OIDC `/userinfo` discovery against
/// the stub, testing the ladder instead of the link.
fn sign_idp_access_token(issuer: &str, sub: &str, email: &str) -> String {
    let key = EncodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.key"))
        .expect("load test RSA private key");
    let now = now_unix();
    let claims = LinkJwtClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: issuer.to_string(),
        aud: common::TEST_AUDIENCE.to_string(),
        iat: now,
        exp: now + 3600,
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).expect("sign IdP JWT")
}

/// `refresh_token` is an `Option` so the stub can model the misconfigured-client case: a 200 that
/// omits it entirely, which is what an IdP with `offline_access` (or rotation) not enabled on the
/// link client actually returns. `skip_serializing_if` matters — a JSON `null` is a different wire
/// shape from an absent key, and it is absence the real misconfiguration produces.
#[derive(Debug, Serialize)]
struct StubTokenResponse {
    access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    expires_in: u64,
}

/// Mount the RFC 6749 token endpoint on the stub, returning `access_token` on any exchange.
async fn stub_token_endpoint(app: &SlackLinkApp, access_token: String) {
    mount_token_endpoint(app, access_token, Some(STUB_REFRESH_TOKEN.to_string())).await;
}

/// The same endpoint, but answering WITHOUT a refresh token — the misconfigured-link-client shape.
async fn stub_token_endpoint_without_refresh_token(app: &SlackLinkApp, access_token: String) {
    mount_token_endpoint(app, access_token, None).await;
}

async fn mount_token_endpoint(
    app: &SlackLinkApp,
    access_token: String,
    refresh_token: Option<String>,
) {
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(StubTokenResponse {
            access_token,
            refresh_token,
            expires_in: 86400,
        }))
        .mount(&app.idp)
        .await;
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs() as i64
}

/// POST `/internal/slack/link-state` as the mention agent would: HMAC over the RAW body.
///
/// The signature is produced by `temper_core::internal_sig::sign` — the same function the
/// gate verifies with — rather than re-derived here. A hand-rolled MAC would be testing this
/// file's idea of the scheme against itself.
async fn post_link_state(
    app: &SlackLinkApp,
    principal: &str,
    signature: Option<String>,
) -> reqwest::Response {
    let body = format!(r#"{{"slack_principal_id":"{principal}"}}"#);
    let ts = now_unix();
    let sig = signature.unwrap_or_else(|| {
        temper_core::internal_sig::sign(SLACK_SECRET.as_bytes(), ts, body.as_bytes())
    });

    app.http
        .post(format!("http://{}/internal/slack/link-state", app.addr))
        .header("Content-Type", "application/json")
        .header(temper_core::internal_sig::TIMESTAMP_HEADER, ts.to_string())
        .header(temper_core::internal_sig::SIGNATURE_HEADER, sig)
        .body(body)
        .send()
        .await
        .expect("post link state")
}

/// Mint an intent for [`SLACK_PRINCIPAL`] and return the opaque `state` the IdP would echo back.
async fn mint_state_nonce(app: &SlackLinkApp) -> String {
    mint_state_nonce_for(app, SLACK_PRINCIPAL).await
}

/// Mint an intent for an arbitrary principal. Needed by the two-workspaces test, where one human
/// legitimately holds a distinct principal per Slack workspace.
async fn mint_state_nonce_for(app: &SlackLinkApp, principal: &str) -> String {
    let res = post_link_state(app, principal, None).await;
    assert_eq!(
        res.status(),
        200,
        "a correctly signed intent must be minted"
    );

    let body: serde_json::Value = res.json().await.expect("intent response is JSON");
    assert_eq!(
        body["status"], "unlinked",
        "an unlinked principal must get the unlinked arm: {body}"
    );
    let authorize_url = body["authorize_url"]
        .as_str()
        .expect("the response carries an authorize_url");

    let url = reqwest::Url::parse(authorize_url).expect("authorize_url is a URL");
    url.query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .expect("the authorize URL carries the opaque state")
}

/// Provision a profile through the REAL auto-provisioning path — an authenticated
/// `GET /api/profile`, exactly as a first sign-in does. Returns the token used, so a
/// caller that needs to keep authenticating as this user (e.g. driving the CLI) can.
///
/// Deliberately not a hand-written INSERT: the row this flow must find is the row the login
/// path writes, and only the login path knows its full shape (profile + auth link under
/// `PROVIDER` + the surface emitters). A fixture INSERT would let the two drift.
async fn provision_profile(app: &SlackLinkApp, sub: &str, email: &str) -> String {
    let token = sign_idp_access_token(&app.issuer(), sub, email);
    let res = app
        .http
        .get(format!("http://{}/api/profile", app.addr))
        .bearer_auth(&token)
        .send()
        .await
        .expect("GET /api/profile");
    assert_eq!(
        res.status(),
        200,
        "the token user must auto-provision on first authenticated request"
    );
    token
}

async fn count_profiles(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
        .fetch_one(pool)
        .await
        .expect("count profiles")
}

async fn count_slack_links(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider = 'slack'")
        .fetch_one(pool)
        .await
        .expect("count slack links")
}

/// The profile the login path minted for `sub`, found the way every predicate finds it: the
/// auth-link row under the SERVER's configured provider name.
async fn profile_id_for_sub(pool: &PgPool, sub: &str) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links \
         WHERE auth_provider = $1 AND auth_provider_user_id = $2",
    )
    .bind(PROVIDER)
    .bind(sub)
    .fetch_one(pool)
    .await
    .expect("the provisioned profile has an auth link")
}

/// Does a Slack link row exist for this exact principal? Matched WHOLE, never split.
async fn slack_link_exists(pool: &PgPool, principal: &str) -> bool {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_profile_auth_links \
         WHERE auth_provider = 'slack' AND auth_provider_user_id = $1",
    )
    .bind(principal)
    .fetch_one(pool)
    .await
    .expect("count the principal's link rows");
    n > 0
}

/// POST the admin disconnect endpoint as `token`.
async fn post_admin_disconnect(
    app: &SlackLinkApp,
    token: &str,
    principal: &str,
) -> reqwest::Response {
    app.http
        .post(format!(
            "http://{}/api/admin/slack/links/disconnect",
            app.addr
        ))
        .bearer_auth(token)
        // The real wire type, not an inline `json!` that could drift from it.
        .json(&temper_core::types::slack::SlackDisconnectRequest {
            slack_principal_id: principal.to_string(),
        })
        .send()
        .await
        .expect("post admin disconnect")
}

async fn count_intents(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_slack_link_intents")
        .fetch_one(pool)
        .await
        .expect("count intents")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// **D3, the load-bearing invariant.** Connecting Slack is not a registration route.
///
/// The state and the code are both valid — the flow gets all the way to resolution — and the
/// token names an identity no `kb_profile_auth_links` row knows. It must refuse WITHOUT
/// minting anything.
///
/// The refusal message alone is not the assertion: a regression that swaps
/// `authenticate_token_existing_only` for `authenticate_token` creates the profile and then
/// fails somewhere downstream would still render a "not connected" page. The profile count
/// is what actually holds the line, and the auto-join trigger on that INSERT is why it
/// matters — a stray click would confer real team reach.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn callback_with_an_unknown_identity_creates_no_profile(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    // An identity with no auth-link row AND no email that could reconcile onto an existing
    // profile — both rungs of the resolve must miss, or the test would pass by the wrong door.
    let token = sign_idp_access_token(
        &app.issuer(),
        "idp-sub-with-no-temper-profile",
        "nobody-e0d5c9@unlinked.invalid",
    );
    stub_token_endpoint(&app, token).await;

    let state_nonce = mint_state_nonce(&app).await;
    let before = count_profiles(&pool).await;

    let res = app
        .http
        .get(app.callback_url(&state_nonce))
        .send()
        .await
        .expect("GET callback");

    assert_eq!(
        res.status(),
        200,
        "the callback always renders a page — a human is looking at it"
    );
    let body = res.text().await.expect("callback body");
    assert!(
        body.contains("No temper account is linked"),
        "the refusal must be the lookup-only one: {body}"
    );

    assert_eq!(
        count_profiles(&pool).await,
        before,
        "lookup-only must not mint a profile"
    );
    assert_eq!(
        count_slack_links(&pool).await,
        0,
        "a refused link must write no directory row"
    );
}

/// **D6, the single-use invariant, end to end.**
///
/// The intent burn is an atomic conditional UPDATE, so the second callback with the SAME URL
/// must find nothing — even though its code and state are byte-identical to the first's.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_replayed_state_is_rejected(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    let sub = "idp-sub-with-a-temper-profile";
    let email = "linker-7f21a4@example.invalid";
    provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    let url = app.callback_url(&mint_state_nonce(&app).await);

    let first = app
        .http
        .get(&url)
        .send()
        .await
        .expect("first callback")
        .text()
        .await
        .expect("first body");
    assert!(
        first.contains("Linked as"),
        "the first callback must link: {first}"
    );
    assert_eq!(
        count_slack_links(&pool).await,
        1,
        "the link is written once"
    );

    let second = app
        .http
        .get(&url)
        .send()
        .await
        .expect("second callback")
        .text()
        .await
        .expect("second body");
    assert!(
        second.contains("expired or was already used"),
        "a replayed state must be refused: {second}"
    );
    assert_eq!(
        count_slack_links(&pool).await,
        1,
        "the replay must not write a second row"
    );
}

/// Re-linking the same Slack user is idempotent — two whole intents + callbacks for the SAME
/// principal and profile leave exactly one directory row.
///
/// Distinct from the replay test: both flows here are legitimate and both succeed. The single
/// row is `UNIQUE(auth_provider, auth_provider_user_id)` doing its job (D4), not the intent
/// burn refusing the second attempt.
///
/// **Both intents are minted BEFORE either callback runs**, and that ordering is forced rather
/// than stylistic: `link-state` answers `linked` once the first callback lands, so a
/// mint-callback-mint-callback loop could never obtain the second URL. This is the real
/// scenario the ordering models — a user mentions twice, then clicks both links — and it is
/// the only route to a second callback now that a linked principal is never issued a
/// challenge. Consequence worth naming: re-link is no longer reachable by mentioning again.
/// The upsert stays idempotent for the concurrent case above; a deliberate "connect a
/// different account" affordance is a separate feature, not a side effect of the old bug.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn relinking_the_same_principal_is_idempotent(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    let sub = "idp-sub-relinker";
    let email = "relinker-3c88b0@example.invalid";
    provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    // Two mentions while still unlinked => two live intents, two distinct states.
    let first_url = app.callback_url(&mint_state_nonce(&app).await);
    let second_url = app.callback_url(&mint_state_nonce(&app).await);

    for (attempt, url) in [(1, &first_url), (2, &second_url)] {
        let body = app
            .http
            .get(url)
            .send()
            .await
            .expect("callback")
            .text()
            .await
            .expect("body");
        assert!(
            body.contains("Linked as"),
            "link attempt {attempt} must succeed: {body}"
        );
    }

    assert_eq!(
        count_slack_links(&pool).await,
        1,
        "re-linking the same principal must upsert, not duplicate"
    );
}

/// **There is no rebind, end to end.** A callback whose token resolves to a DIFFERENT profile
/// than the principal's existing link is refused, and the original link stands.
///
/// This is the security half of the design, at the tier where it is real. The residual
/// URL-theft threat is: steal the victim's ephemeral link message, complete the login as
/// yourself, and bind their principal to YOUR profile — every future `@temper` from the victim
/// then writes into your KB. Refusing here closes that attack for an already-linked victim
/// outright (and D9 means they are never issued a URL to steal in the first place).
///
/// The page text is not the load-bearing assertion — the row is. A regression that renders the
/// refusal while the write still lands would pass a text-only check and leak exactly what the
/// refusal exists to prevent. So: the link must still point at the ORIGINAL profile.
///
/// Both intents are minted before either callback runs, for the same forced reason as the
/// idempotency test above: `link-state` answers `linked` once the first callback lands, so
/// there is no other route to a second callback.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_callback_for_a_principal_linked_to_another_profile_is_refused(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    let victim_sub = "idp-sub-victim";
    let victim_email = "victim-4a19c2@example.invalid";
    let attacker_sub = "idp-sub-attacker";
    let attacker_email = "attacker-8d02fe@example.invalid";
    provision_profile(&app, victim_sub, victim_email).await;
    provision_profile(&app, attacker_sub, attacker_email).await;

    // Two intents for the SAME principal, minted while it is still unlinked.
    let victim_url = app.callback_url(&mint_state_nonce(&app).await);
    let attacker_url = app.callback_url(&mint_state_nonce(&app).await);

    // The victim links first. The stub hands back the victim's token.
    stub_token_endpoint(
        &app,
        sign_idp_access_token(&app.issuer(), victim_sub, victim_email),
    )
    .await;
    let first = app
        .http
        .get(&victim_url)
        .send()
        .await
        .expect("victim callback")
        .text()
        .await
        .expect("victim body");
    assert!(first.contains("Linked as"), "the victim must link: {first}");

    let victim_profile_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links \
         WHERE auth_provider = 'slack' AND auth_provider_user_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("the victim's link row exists");

    // Now the attacker completes the stolen URL, authenticating as THEMSELVES.
    //
    // `reset()` is load-bearing, not tidiness: wiremock sorts mounted mocks by priority with a
    // STABLE sort and every mock here has the default priority, so the FIRST-registered match
    // wins forever (`mock_set.rs:63-68`). Simply mounting a second `/oauth/token` stub would
    // NOT shadow the victim's — the exchange would keep returning the victim's token, the
    // callback would resolve to the victim's own profile, and this test would report a green
    // "no rebind" while never once attempting one.
    app.idp.reset().await;
    stub_token_endpoint(
        &app,
        sign_idp_access_token(&app.issuer(), attacker_sub, attacker_email),
    )
    .await;
    let res = app
        .http
        .get(&attacker_url)
        .send()
        .await
        .expect("attacker callback");
    assert_eq!(
        res.status(),
        200,
        "the callback always renders a page — a human is looking at it"
    );
    let second = res.text().await.expect("attacker body");

    assert!(
        second.contains("already connected to a different temper account"),
        "the rebind must be refused, and say so: {second}"
    );
    assert!(
        !second.contains("Linked as"),
        "the refusal must not render the success page: {second}"
    );

    assert_eq!(
        count_slack_links(&pool).await,
        1,
        "the refusal must not add a row"
    );
    let after: uuid::Uuid = sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links \
         WHERE auth_provider = 'slack' AND auth_provider_user_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("the link row survives");
    assert_eq!(
        after, victim_profile_id,
        "the link must STILL point at the original profile — a refusal that renders the right \
         page while the write lands is the exact bug this test exists to catch",
    );
}

/// **The re-prompt regression.** A linked user must be told they are linked — and must cost
/// nothing to tell.
///
/// The endpoint's question is "what do I say to this person?", not "mint me a URL". Before
/// this branch existed, every mention from an already-linked user minted a fresh intent and
/// answered with a link challenge, so a user who successfully connected was asked to connect
/// again on their very next mention, forever.
///
/// The intent count is the load-bearing assertion, not the status. Asserting only
/// `status == "linked"` would pass a regression that answers correctly and *still* mints the
/// junk row on the way — the waste would be invisible and unbounded, one row per mention per
/// linked user. So: count before, count after, unchanged.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_linked_principal_gets_its_handle_and_mints_no_intent(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    // Establish the link through the REAL flow — provision, mint, callback — rather than a
    // fixture INSERT. The row this lookup must find is the row the callback writes.
    let sub = "idp-sub-already-linked";
    let email = "linked-91b3ef@example.invalid";
    provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    let first = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(first.contains("Linked as"), "setup must link: {first}");

    let intents_after_linking = count_intents(&pool).await;

    // The next mention. This is the call that used to re-prompt.
    let res = post_link_state(&app, SLACK_PRINCIPAL, None).await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.expect("link-state response is JSON");

    assert_eq!(
        body["status"], "linked",
        "an already-linked principal must not be asked to link again: {body}"
    );
    assert!(
        body["authorize_url"].is_null(),
        "the linked arm carries no challenge — the union has no such field: {body}"
    );

    // The handle is the profile's slug (`kb_profiles.handle`), not the Slack id.
    let handle = body["handle"]
        .as_str()
        .expect("the linked arm names a handle");
    let expected: String = sqlx::query_scalar(
        "SELECT p.handle FROM kb_profile_auth_links l \
         JOIN kb_profiles p ON p.id = l.profile_id \
         WHERE l.auth_provider = 'slack' AND l.auth_provider_user_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("the link row names a profile");
    assert_eq!(handle, expected, "the handle must name the linked profile");

    assert_eq!(
        count_intents(&pool).await,
        intents_after_linking,
        "a linked principal must mint NO intent — this is the junk-row regression"
    );
}

/// The HMAC gate on the intent route, asserted rather than assumed.
///
/// This gate is the whole reason Slack-side hijack is expensive. Slack user ids are visible
/// to any workspace member, so an ungated intent endpoint would let anyone mint a link URL
/// for anyone else's principal and bind it to their own profile. The gate reduces the attack
/// from "read a public id" to "steal an ephemeral message only the victim can see" — so a
/// forged signature must be refused, and it must leave no intent behind for a later guess to
/// land on.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn link_state_rejects_a_forged_signature(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    // Well-formed lowercase hex of the right length, signed with the WRONG key — the shape a
    // real forgery has. A malformed string would prove only that hex decoding fails.
    let forged = temper_core::internal_sig::sign(
        b"not-the-slack-link-secret",
        now_unix(),
        format!(r#"{{"slack_principal_id":"{SLACK_PRINCIPAL}"}}"#).as_bytes(),
    );

    let res = post_link_state(&app, SLACK_PRINCIPAL, Some(forged)).await;
    assert_eq!(res.status(), 401, "a forged signature must be refused");
    assert_eq!(
        count_intents(&pool).await,
        0,
        "a refused call must mint no intent"
    );

    // The gate must also refuse a caller who simply omits the signature rather than forging
    // one — `require_signature_with` reads the headers before it reads the body, and a
    // missing header must land in the same refusal, not skip the check.
    let body = format!(r#"{{"slack_principal_id":"{SLACK_PRINCIPAL}"}}"#);
    let res = app
        .http
        .post(format!("http://{}/internal/slack/link-state", app.addr))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("post unsigned link state");
    assert_eq!(res.status(), 401, "an unsigned call must be refused");
    assert_eq!(
        count_intents(&pool).await,
        0,
        "an unsigned call must mint no intent"
    );
}

/// **T3, end to end.** A successful link vaults the refresh token the exchange returned —
/// encrypted at rest — and a subsequent mint yields an access token, refreshing against the IdP
/// when the cached one has expired.
///
/// This is the closest a local tier gets to the acceptance criterion "a linked user has an
/// independent encrypted grant; refresh yields a fresh AT": the callback runs the real seam
/// against a real Postgres, `store_grant` seals the stub's `refresh_token`, and `mint_access_token`
/// exercises the FOR UPDATE + decrypt-cached path and then the decrypt-RT → refresh → rotate path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_successful_link_vaults_an_encrypted_grant_that_mints_a_token(pool: PgPool) {
    use temper_services::services::slack_grant_vault_service::{mint_access_token, MintOutcome};

    let app = setup_slack_app(&pool).await;
    let sub = "idp-sub-vaulted";
    let email = "vaulted-5b12aa@example.invalid";
    provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    let body = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(body.contains("Linked as"), "the link must succeed: {body}");

    // D-A: the mint gate is `standing = 'approved'`, and the callback births `Denied` — approve so
    // the happy-path mint below returns a token rather than `Revoked`.
    approve_standing(&pool, profile_id_for_sub(&pool, sub).await).await;

    // The RT is stored ENCRYPTED — the raw column must not equal the stub's plaintext token.
    let rt_ciphertext: Vec<u8> = sqlx::query_scalar(
        "SELECT rt_ciphertext FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("a vault row exists after linking");
    assert_ne!(
        rt_ciphertext,
        STUB_REFRESH_TOKEN.as_bytes(),
        "the refresh token must be sealed at rest, never stored as plaintext",
    );

    let token_url = format!("{}/oauth/token", app.issuer());

    // First mint: the cached AT from the exchange is still valid, so this returns it with no
    // refresh — proving the store → decrypt-cached round-trip through the real DB and crypto.
    match mint_access_token(&pool, &vault_key(), &token_url, CLIENT_ID, SLACK_PRINCIPAL)
        .await
        .expect("mint")
    {
        MintOutcome::Token { access_token, .. } => {
            assert!(
                !access_token.is_empty(),
                "the cached mint must yield a token"
            )
        }
        other => panic!("expected a cached token, got {other:?}"),
    }

    // Expire the cached AT, then mint again: this drives the decrypt-RT → refresh → rotate path
    // against the stub's `/oauth/token`, and rotates the stored RT to the stub's new one.
    sqlx::query("UPDATE kb_slack_grant_vault SET access_expires_at = now() - interval '1 hour' WHERE slack_principal_id = $1")
        .bind(SLACK_PRINCIPAL)
        .execute(&pool)
        .await
        .expect("expire the cached access token");

    match mint_access_token(&pool, &vault_key(), &token_url, CLIENT_ID, SLACK_PRINCIPAL)
        .await
        .expect("refreshing mint")
    {
        MintOutcome::Token { access_token, .. } => assert!(
            !access_token.is_empty(),
            "the refreshing mint must yield a token"
        ),
        other => panic!("expected a refreshed token, got {other:?}"),
    }

    // After a refresh the cached AT is fresh again — the rotation wrote a new expiry.
    let expires_in_future: bool = sqlx::query_scalar(
        "SELECT access_expires_at > now() FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("read the refreshed expiry");
    assert!(
        expires_in_future,
        "the refresh must re-cache a live access token"
    );
}

// ---------------------------------------------------------------------------
// Disconnect — driven through the REAL `temper` CLI binary, not the service fn
// directly. This is the only tier that exercises the full stack for this
// command: JWT auth → handler → service → the three-table delete, and — just
// as load-bearing — the CLI's own JSON-on-stdout / caveats-on-stderr contract.
// ---------------------------------------------------------------------------

/// **The disconnect happy path, end to end.** Link through the real flow, then unbind through
/// the real CLI, and prove it by absence of rows — not by the CLI's own say-so.
///
/// Two things this test is careful about:
///
/// 1. It asserts on `kb_profile_auth_links`, `kb_slack_grant_vault` and
///    `kb_slack_link_intents` directly. A test that only checked `out.status.success()` or the
///    JSON payload's claims would pass against a handler that deleted nothing — the CLI would
///    just be reporting what it wished had happened.
/// 2. `serde_json::from_slice(&out.stdout)` must succeed. That is this command's stdout
///    contract, asserted rather than assumed: with `--format json` the ONLY thing on stdout is
///    the payload, and every caveat the command emits goes to stderr. Any future caveat wired to
///    stdout — a `println!`, a helper that does not route through `output::` — breaks the parse
///    here. Do not relax this to a substring check: substring matching would still pass with
///    prose prepended to the JSON, which is the exact failure it exists to catch.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn disconnect_unbinds_the_principal_and_the_next_mention_prompts_to_link(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    let sub = "idp-sub-disconnect";
    let email = "disconnect-1a2b3c@example.invalid";
    let token = provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    // Link.
    let body = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(body.contains("Linked as"), "the link must succeed: {body}");
    assert_eq!(count_slack_links(&pool).await, 1);

    // Disconnect via the real CLI binary, authenticated as the linked user.
    let out = common::run_temper_cli_with_token(
        &format!("http://{}", app.addr),
        &token,
        &app.cli_config_path(),
        &["slack", "disconnect", "--format", "json"],
    )
    .await
    .expect("cli disconnect");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Must parse as JSON on its own — see the module note above.
    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        parsed["disconnected"][0]["slack_principal_id"], SLACK_PRINCIPAL,
        "the response must NAME the principal it unbound: {parsed}"
    );
    assert_eq!(
        parsed["disconnected"].as_array().map(Vec::len),
        Some(1),
        "exactly one principal was linked, so exactly one entry: {parsed}"
    );

    // Assert absence of the rows, not the success message.
    assert_eq!(
        count_slack_links(&pool).await,
        0,
        "identity row must be gone"
    );
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("count grant vault rows");
    assert_eq!(grants, 0, "the sealed grant must be destroyed");
    assert_eq!(count_intents(&pool).await, 0, "intents must be swept");

    // The next mention is offered a fresh link — the normal T2 flow, no special path.
    let res = post_link_state(&app, SLACK_PRINCIPAL, None).await;
    assert_eq!(res.status(), 200);
    let state: serde_json::Value = res.json().await.expect("json");
    assert_eq!(
        state["status"], "unlinked",
        "a disconnected principal must be offered a link again"
    );
}

/// Disconnect is not deactivation, and running it twice is not an error.
///
/// The profile row — and everything hanging off it (teams, resources) — must survive a
/// disconnect untouched; only the identity binding, grant and intents are in scope. The second
/// invocation finds nothing left to unbind (an empty `disconnected` list is the CLI's own no-op
/// arm) but must still exit success, not fail because the first call already cleaned up.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn disconnect_leaves_the_profile_and_is_idempotent(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    let sub = "idp-sub-idempotent";
    let email = "idempotent-9f8e7d@example.invalid";
    let token = provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    let body = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(body.contains("Linked as"), "the link must succeed: {body}");

    let profiles_before = count_profiles(&pool).await;

    for attempt in 0..2 {
        let out = common::run_temper_cli_with_token(
            &format!("http://{}", app.addr),
            &token,
            &app.cli_config_path(),
            &["slack", "disconnect", "--format", "json"],
        )
        .await
        .expect("cli disconnect");
        assert!(
            out.status.success(),
            "attempt {attempt} must succeed; stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );

        let parsed: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
        let n = parsed["disconnected"]
            .as_array()
            .map(Vec::len)
            .expect("the response always carries a `disconnected` array");
        if attempt == 0 {
            assert_eq!(n, 1, "the first call unbinds a real link: {parsed}");
        } else {
            assert_eq!(
                n, 0,
                "the second call finds nothing left to unbind — an EMPTY list, not an error: \
                 {parsed}"
            );
        }
    }

    assert_eq!(
        count_profiles(&pool).await,
        profiles_before,
        "disconnect is not deactivation — the profile must survive"
    );
}

// ---------------------------------------------------------------------------
// The ADMIN disconnect arm.
//
// This is the arm with a real authorization gate AND a caller-supplied
// principal — the only place in the disconnect surface where naming someone
// else's identity is even expressible. It shipped with no test at any tier.
// ---------------------------------------------------------------------------

/// **The admin gate, asserted on the ROW.** A non-admin naming someone else's principal is
/// refused, and the victim's link survives.
///
/// `is_system_admin` is the whole gate here: the gated router admits everyone under
/// `access_mode = 'open'` (which is what production runs), so if this check regressed there
/// would be nothing behind it. Any authenticated user could then unbind any Slack identity in
/// the instance by naming a principal — and Slack user ids are visible to every workspace
/// member, so the principal is not a secret.
///
/// **Why the row assertion is load-bearing:** a regression that moved the admin check AFTER the
/// service call — or that returned 403 from a later branch — would still answer 403 while
/// having already destroyed the binding. Asserting the status alone would report green on
/// exactly that bug. So the link row must still be there afterwards.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_disconnect_refuses_a_non_admin_and_leaves_the_link_intact(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    // The victim links through the real flow.
    let victim_sub = "idp-sub-admin-victim";
    let victim_email = "admin-victim-2f4e6a@example.invalid";
    provision_profile(&app, victim_sub, victim_email).await;
    stub_token_endpoint(
        &app,
        sign_idp_access_token(&app.issuer(), victim_sub, victim_email),
    )
    .await;
    let body = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(body.contains("Linked as"), "the victim must link: {body}");
    assert!(slack_link_exists(&pool, SLACK_PRINCIPAL).await);

    // A DIFFERENT, ordinary user — authenticated, but not a system admin.
    let intruder_token = provision_profile(
        &app,
        "idp-sub-admin-intruder",
        "admin-intruder-7c1d90@example.invalid",
    )
    .await;

    let res = post_admin_disconnect(&app, &intruder_token, SLACK_PRINCIPAL).await;
    assert_eq!(
        res.status(),
        403,
        "a non-admin must be refused the admin disconnect arm"
    );

    assert!(
        slack_link_exists(&pool, SLACK_PRINCIPAL).await,
        "the refusal must leave the victim's link row STANDING — a 403 returned after the \
         deletes would pass a status-only assertion while doing the exact damage the gate exists \
         to prevent",
    );
}

/// The admin arm actually unbinds — asserted by absence of the row, not by the response's claim.
///
/// The companion to the refusal above: a gate that refused everyone would pass that test and be
/// useless. This one proves the admitted path does the work, and that the response NAMES the
/// principal it acted on (the uniform `disconnected` shape both surfaces return).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_disconnect_unbinds_the_named_principal(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    let victim_sub = "idp-sub-admin-target";
    let victim_email = "admin-target-5e8b21@example.invalid";
    provision_profile(&app, victim_sub, victim_email).await;
    stub_token_endpoint(
        &app,
        sign_idp_access_token(&app.issuer(), victim_sub, victim_email),
    )
    .await;
    let body = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(body.contains("Linked as"), "the target must link: {body}");

    // The operator: a separate profile, promoted to OWNER of the gating team.
    let admin_sub = "idp-sub-the-operator";
    let admin_token = provision_profile(&app, admin_sub, "operator-3d70cc@example.invalid").await;
    common::make_system_admin(&pool, profile_id_for_sub(&pool, admin_sub).await).await;

    let res = post_admin_disconnect(&app, &admin_token, SLACK_PRINCIPAL).await;
    assert_eq!(res.status(), 200, "a system admin must be admitted");
    let payload: serde_json::Value = res.json().await.expect("json body");
    assert_eq!(
        payload["disconnected"][0]["slack_principal_id"], SLACK_PRINCIPAL,
        "the response must name the principal it unbound: {payload}"
    );

    assert!(
        !slack_link_exists(&pool, SLACK_PRINCIPAL).await,
        "the admin disconnect must actually delete the identity row"
    );
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("count grants");
    assert_eq!(grants, 0, "the sealed grant must be destroyed too");
}

/// A malformed principal is a 400, and it is the VALIDATOR that says so.
///
/// The principal is caller-supplied on this arm, so `validate_slack_principal` is the only thing
/// between an operator's typo and a query keyed on arbitrary text. 400 is also a documented
/// status on this path now — this test is what keeps that documentation honest.
///
/// **Why it bites:** the caller is a real system admin, so a 403 here would mean the gate fired
/// for the wrong reason and a 200 would mean validation was skipped entirely. Only a 400
/// passes, and only if the validator is still in the path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_disconnect_rejects_a_malformed_principal(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    let admin_sub = "idp-sub-typo-operator";
    let admin_token = provision_profile(&app, admin_sub, "typo-op-a91f04@example.invalid").await;
    common::make_system_admin(&pool, profile_id_for_sub(&pool, admin_sub).await).await;

    // Wrong provider prefix, and a bare id with no prefix at all — the two shapes a typo takes.
    for bad in ["discord:T123:U456", "U0BH6A3L6JF"] {
        let res = post_admin_disconnect(&app, &admin_token, bad).await;
        assert_eq!(
            res.status(),
            400,
            "{bad:?} must be refused as malformed, not accepted or mistaken for an authz failure"
        );
    }
}

/// **The reaper's entire security story, asserted.**
///
/// `/api/slack/intents/reap` is mounted on the bare internal router with NO auth middleware —
/// the bearer-secret check inside the handler is all there is. An unauthenticated caller could
/// otherwise delete every consumed and expired link intent in the instance on demand.
///
/// **Why this bites:** the harness sets `embed_dispatch_secret`, so the endpoint is genuinely
/// ENABLED. Deleting `require_dispatch_secret` from the handler turns the first two arms into
/// 200s. And the third arm (correct secret → 200) is what stops the test from passing for the
/// wrong reason: without it, an endpoint that 401'd unconditionally — or that was never
/// mounted, and 404'd — would look identical to a working gate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_intents_reaper_requires_the_bearer_secret(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    let url = format!("http://{}/api/slack/intents/reap", app.addr);

    let unauthenticated = app
        .http
        .get(&url)
        .send()
        .await
        .expect("get reap, no bearer");
    assert_eq!(
        unauthenticated.status(),
        401,
        "an unauthenticated caller must not be able to sweep intents"
    );

    let wrong = app
        .http
        .get(&url)
        .bearer_auth("not-the-reap-secret")
        .send()
        .await
        .expect("get reap, wrong bearer");
    assert_eq!(wrong.status(), 401, "a wrong secret must be refused");

    let right = app
        .http
        .get(&url)
        .bearer_auth(REAP_SECRET)
        .send()
        .await
        .expect("get reap, correct bearer");
    assert_eq!(
        right.status(),
        200,
        "the correct secret must be ADMITTED — otherwise the two 401s above prove nothing about \
         the gate, only that the route is unreachable"
    );
}

/// **Two workspaces, one human, ONE disconnect — and nothing survives it.**
///
/// `kb_profile_auth_links` carries `UNIQUE(auth_provider, auth_provider_user_id)` and nothing
/// keyed on `(profile_id, auth_provider)`, so a person in two Slack workspaces ends up with two
/// rows on the same profile. That is not corruption: the already-linked refusal keys on the
/// *principal*, so the second workspace links through the normal flow, as this test does.
///
/// The self-serve arm used to derive the principal with an unordered, unlimited query fed to
/// `fetch_optional` — which takes the first streamed row and silently discards the rest. So one
/// arbitrary binding was cut, the call reported success, and the OTHER grant stayed live and
/// kept minting act-as-the-human access tokens for a user who had just been told they were
/// disconnected.
///
/// **Why this bites:** it asserts ZERO rows across all three tables after ONE disconnect. Under
/// the old `fetch_optional` exactly one of the two principals survives in every table, so every
/// count is 1, not 0. Counting rows rather than reading the response also means a handler that
/// *claimed* both while cutting one cannot pass.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn disconnect_unbinds_every_principal_the_profile_holds(pool: PgPool) {
    /// The same human's principal in a SECOND Slack workspace: different team id, and a
    /// different user id too (Slack user ids are per-workspace).
    const SECOND_PRINCIPAL: &str = "slack:T99SECONDWS:U99SECONDUSR";

    let app = setup_slack_app(&pool).await;
    let sub = "idp-sub-two-workspaces";
    let email = "two-workspaces-6b0d4f@example.invalid";
    let token = provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    // Link both principals to the SAME profile, each through the real flow.
    for principal in [SLACK_PRINCIPAL, SECOND_PRINCIPAL] {
        let nonce = mint_state_nonce_for(&app, principal).await;
        let body = app
            .http
            .get(app.callback_url(&nonce))
            .send()
            .await
            .expect("callback")
            .text()
            .await
            .expect("callback body");
        assert!(body.contains("Linked as"), "{principal} must link: {body}");
    }
    assert_eq!(
        count_slack_links(&pool).await,
        2,
        "both workspaces' principals must be linked before the disconnect — if this is 1, the \
         setup never reached the state under test and the assertions below are vacuous",
    );
    let profile_id = profile_id_for_sub(&pool, sub).await;
    let links_on_profile: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_profile_auth_links \
         WHERE auth_provider = 'slack' AND profile_id = $1",
    )
    .bind(profile_id)
    .fetch_one(&pool)
    .await
    .expect("count this profile's slack links");
    assert_eq!(
        links_on_profile, 2,
        "both must hang off the SAME profile — that is the whole premise"
    );

    // ONE self-serve disconnect, through the real CLI.
    let out = common::run_temper_cli_with_token(
        &format!("http://{}", app.addr),
        &token,
        &app.cli_config_path(),
        &["slack", "disconnect", "--format", "json"],
    )
    .await
    .expect("cli disconnect");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    let named: Vec<&str> = parsed["disconnected"]
        .as_array()
        .expect("a `disconnected` array")
        .iter()
        .map(|d| d["slack_principal_id"].as_str().expect("a principal"))
        .collect();
    assert_eq!(
        named.len(),
        2,
        "the response must NAME both principals it acted on: {parsed}"
    );
    assert!(named.contains(&SLACK_PRINCIPAL) && named.contains(&SECOND_PRINCIPAL));

    // The load-bearing half: ZERO live rows in all three tables.
    assert_eq!(
        count_slack_links(&pool).await,
        0,
        "no Slack identity row may survive — a surviving one is a live binding the user believes \
         is gone",
    );
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_slack_grant_vault")
        .fetch_one(&pool)
        .await
        .expect("count grants");
    assert_eq!(
        grants, 0,
        "no sealed grant may survive — a surviving one still mints act-as-the-human tokens",
    );
    assert_eq!(count_intents(&pool).await, 0, "no intent may survive");
}

// ---------------------------------------------------------------------------------------------
// T4 — the mint route (`POST /internal/slack/mint`)
//
// Every test below drives the ROUTE, never `mint_access_token` directly. That is deliberate and
// load-bearing: the rule these endpoints enforce is *"naming a principal must not be sufficient
// to mint its token,"* and the only thing enforcing it is the signature layer the route is
// mounted behind. A test that called the service function would bypass the gate entirely and
// prove nothing about authorization — the exact shape of vacuous gate this repo has been bitten
// by before.
// ---------------------------------------------------------------------------------------------

/// POST `/internal/slack/mint` as the mention agent would: HMAC over the RAW body, keyed on
/// [`MINT_SECRET`].
///
/// `secret` is a parameter rather than a constant so the cross-key probe can sign with the WRONG
/// key without hand-rolling the scheme.
async fn post_mint(
    app: &SlackLinkApp,
    principal: &str,
    secret: &[u8],
    signature: Option<String>,
) -> reqwest::Response {
    let body = format!(r#"{{"slack_principal_id":"{principal}"}}"#);
    let ts = now_unix();
    let sig =
        signature.unwrap_or_else(|| temper_core::internal_sig::sign(secret, ts, body.as_bytes()));

    app.http
        .post(format!("http://{}/internal/slack/mint", app.addr))
        .header("Content-Type", "application/json")
        .header(temper_core::internal_sig::TIMESTAMP_HEADER, ts.to_string())
        .header(temper_core::internal_sig::SIGNATURE_HEADER, sig)
        .body(body)
        .send()
        .await
        .expect("post mint")
}

/// Link [`SLACK_PRINCIPAL`] through the real callback so a sealed grant exists to mint from, and
/// **approve** the resulting principal.
///
/// Under D-A the mint gate is `standing = 'approved'`, and the callback births every principal
/// `Denied` (D11), so a vaulted principal that can actually mint is an approved one — approved here
/// as an admin would in reality. The 401 / not-vaulted / revoked-grant callers are unaffected:
/// those outcomes are decided before, or regardless of, standing.
async fn link_a_vaulted_principal(app: &SlackLinkApp, pool: &PgPool) {
    let sub = "idp-sub-mint";
    let email = "mint-7c41de@example.invalid";
    provision_profile(app, sub, email).await;
    stub_token_endpoint(app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    let body = app
        .http
        .get(app.callback_url(&mint_state_nonce(app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(body.contains("Linked as"), "the link must succeed: {body}");

    approve_standing(pool, profile_id_for_sub(pool, sub).await).await;
}

/// Grant a principal `approved` standing — the mint gate (`state = 'approved'`, D-A) and the
/// Level-1 auth gate both read this now that Phase 2 dropped `kb_profiles.is_active`.
async fn approve_standing(pool: &PgPool, profile_id: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'approved') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'approved', updated = now()",
    )
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("approve the principal's standing");
}

/// The happy path, through the gate: a vaulted principal mints a token with an expiry.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mint_returns_a_token_for_a_vaulted_principal(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    link_a_vaulted_principal(&app, &pool).await;

    let res = post_mint(&app, SLACK_PRINCIPAL, MINT_SECRET.as_bytes(), None).await;
    assert_eq!(res.status(), 200, "a signed mint for a vaulted principal");

    let body: serde_json::Value = res.json().await.expect("mint body");
    assert_eq!(body["status"], "token");
    assert!(
        body["access_token"].as_str().is_some_and(|t| !t.is_empty()),
        "the token arm must carry a non-empty access token: {body}",
    );

    // The expiry must be in the FUTURE, in milliseconds. A seconds-valued timestamp would still
    // be "a number" and still deserialize — it would just tell eve the token expired in 1970 and
    // make it refresh on every single call. Asserting the magnitude is what catches the unit.
    let expires_at_ms = body["expires_at_ms"].as_i64().expect("expires_at_ms");
    let now_ms = now_unix() * 1000;
    assert!(
        expires_at_ms > now_ms,
        "expiry must be in the future (ms since epoch); got {expires_at_ms} against now {now_ms} \
         — a value below `now` usually means seconds were sent where ms were expected",
    );
}

/// **The bite probe for the two-key split.** A mint call signed with the LINK-STATE key must be
/// refused.
///
/// This is the test that fails if anyone "simplifies" the config by reusing `SLACK_LINK_SECRET`
/// for both gates. Without it the collapse would be invisible: every other test here would stay
/// green, because they all sign with whatever key the endpoint happens to use.
///
/// What it protects is not hypothetical. Link-state answers *"is this principal linked?"*; the
/// mint endpoint hands back a bearer that resolves to a real profile with that human's FULL reach
/// — `resources_visible_to` takes a profile and nothing else, so there is no narrowing behind it.
/// One shared key would make compromise of the cheap capability yield the expensive one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mint_refuses_the_link_state_key(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    link_a_vaulted_principal(&app, &pool).await;

    // Signed correctly — with the WRONG secret. This is a well-formed signature by a caller who
    // legitimately holds the link-state key and nothing more.
    let res = post_mint(&app, SLACK_PRINCIPAL, SLACK_SECRET.as_bytes(), None).await;
    assert_eq!(
        res.status(),
        401,
        "the link-state key must NOT open the mint endpoint — holding the cheap capability must \
         never confer the act-as-the-human one",
    );

    // And the converse, so the separation is proven in both directions rather than assumed: the
    // mint key must not open link-state either.
    let body = format!(r#"{{"slack_principal_id":"{SLACK_PRINCIPAL}"}}"#);
    let ts = now_unix();
    let sig = temper_core::internal_sig::sign(MINT_SECRET.as_bytes(), ts, body.as_bytes());
    let res = app
        .http
        .post(format!("http://{}/internal/slack/link-state", app.addr))
        .header("Content-Type", "application/json")
        .header(temper_core::internal_sig::TIMESTAMP_HEADER, ts.to_string())
        .header(temper_core::internal_sig::SIGNATURE_HEADER, sig)
        .body(body)
        .send()
        .await
        .expect("post link state with the mint key");
    assert_eq!(
        res.status(),
        401,
        "the mint key must not open link-state either — two keys, two doors",
    );
}

/// A forged signature and an unsigned call are both refused, and neither mints.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mint_rejects_forged_and_unsigned_calls(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    link_a_vaulted_principal(&app, &pool).await;

    let forged = temper_core::internal_sig::sign(
        b"not-the-slack-mint-secret",
        now_unix(),
        format!(r#"{{"slack_principal_id":"{SLACK_PRINCIPAL}"}}"#).as_bytes(),
    );
    let res = post_mint(&app, SLACK_PRINCIPAL, MINT_SECRET.as_bytes(), Some(forged)).await;
    assert_eq!(res.status(), 401, "a forged signature must be refused");

    let res = app
        .http
        .post(format!("http://{}/internal/slack/mint", app.addr))
        .header("Content-Type", "application/json")
        .body(format!(r#"{{"slack_principal_id":"{SLACK_PRINCIPAL}"}}"#))
        .send()
        .await
        .expect("post unsigned mint");
    assert_eq!(res.status(), 401, "an unsigned call must be refused");
}

/// **FINDING D.** An exchange that returns no refresh token must NOT render a success page, and
/// must leave NOTHING behind.
///
/// The IdP answering 200-without-`refresh_token` is a link-client misconfiguration
/// (`offline_access` or refresh-token rotation not enabled). There is nothing to vault, so the
/// link can never mint. The old code fired a `warn!` and returned the slug, rendering
/// "Account connected." — the user was told the thing worked and then hit an unexplained failure
/// at their next mention, with no reason to suspect the link.
///
/// **Why this bites, three ways.** Each assertion falsifies a different half-fix:
///  1. The page must not say "connected" — a fix that only rolled back but still rendered success
///     would pass a row-count-only test.
///  2. `kb_profile_auth_links` must be EMPTY — a fix that only changed the page but left the
///     directory row would pass a page-only test, and leaves the user in the worst state of the
///     three: `lookup_linked_handle` reads that table, so link-state would keep reporting them
///     connected while every mint answers `not_vaulted`, and `link_slack_principal` refuses to
///     rebind, so re-linking cannot repair it. Rolled back, they are cleanly unlinked and one
///     mention away from retrying.
///  3. `kb_slack_grant_vault` must be empty too — the obvious sanity leg.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn callback_without_a_refresh_token_does_not_report_success(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    let sub = "idp-sub-no-refresh-token";
    let email = "no-rt-71c0ad@example.invalid";
    provision_profile(&app, sub, email).await;
    stub_token_endpoint_without_refresh_token(
        &app,
        sign_idp_access_token(&app.issuer(), sub, email),
    )
    .await;

    let res = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("GET callback");
    assert_eq!(
        res.status(),
        200,
        "the callback always renders a page — a human is looking at it"
    );
    let body = res.text().await.expect("callback body");

    assert!(
        !body.contains("Account <em>connected</em>."),
        "a link that can never mint must not render the success page: {body}",
    );
    assert!(
        body.contains("Not <em>connected</em>."),
        "the failure page is what the human must see: {body}",
    );
    assert!(
        body.contains("Nothing was saved."),
        "the page must tell the human the truth about what happened: {body}",
    );

    let links: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("count auth links");
    assert_eq!(
        links, 0,
        "the directory row must be rolled back — a link with no grant is unrepairable, because \
         link-state reads this table and link_slack_principal refuses to rebind",
    );

    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
    )
    .bind(SLACK_PRINCIPAL)
    .fetch_one(&pool)
    .await
    .expect("count grants");
    assert_eq!(grants, 0, "nothing was vaulted, by construction");
}

/// A principal whose link row stands but whose grant was never vaulted mints nothing, and says so
/// distinctly.
///
/// This is not a hypothetical arm, and its provenance is worth stating precisely because this
/// comment used to describe a BUG as though it were the design.
///
/// It once read: "`slack_link.rs` lets the callback succeed when the IdP returns no refresh token
/// — the directory row is written and only a `warn!` fires — so such a user is told 'you're
/// connected' and then cannot mint." That was true, and it was the defect, not the contract: the
/// callback rendered "Account connected." at a user whose link could never mint. The callback now
/// ROLLS BACK that whole link and renders an error page instead
/// (`callback_without_a_refresh_token_does_not_report_success` below), so it no longer manufactures
/// this state.
///
/// The state remains REACHABLE by other routes, which is why mint must still answer it honestly:
/// a user who linked before T3 shipped has a directory row and no vault row at all, and
/// `lookup_linked_handle` reads that table, NOT the vault — so link-state calls them linked while
/// the vault has nothing. The two endpoints must disagree in a way the agent can act on, which is
/// why this is its own status rather than an error or a null token.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mint_reports_not_vaulted_distinctly_from_not_linked(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    link_a_vaulted_principal(&app, &pool).await;

    // Drop the sealed grant while leaving the identity row: exactly the linked-but-unvaulted
    // shape the callback can produce.
    sqlx::query("DELETE FROM kb_slack_grant_vault WHERE slack_principal_id = $1")
        .bind(SLACK_PRINCIPAL)
        .execute(&pool)
        .await
        .expect("drop the vault row");

    // link-state still calls this user linked — the premise of the whole arm.
    let res = post_link_state(&app, SLACK_PRINCIPAL, None).await;
    assert_eq!(res.status(), 200);
    let link_body: serde_json::Value = res.json().await.expect("link-state body");
    assert_eq!(
        link_body["status"], "linked",
        "the premise: an unvaulted user still reads as linked, which is why mint must be honest",
    );

    let res = post_mint(&app, SLACK_PRINCIPAL, MINT_SECRET.as_bytes(), None).await;
    assert_eq!(
        res.status(),
        200,
        "no grant on file is not a transport error"
    );
    let body: serde_json::Value = res.json().await.expect("mint body");
    // The typed refusal: a 200 carrying `refused` with a `not_vaulted` reason — distinct from
    // `not_linked` (no directory row) and from a `standing` refusal (not admitted).
    assert_eq!(body["status"], "refused", "{body}");
    assert_eq!(body["reason"], "not_vaulted", "{body}");
    assert!(
        body["access_token"].is_null(),
        "no token may ride along on a refusal: {body}",
    );
}

// The former `mint_reports_a_revoked_grant_as_revoked` is deleted deliberately, not lost: the
// `revoked_at` disjunct was dropped (spec §2.4) because soft-revoke was superseded by disconnect's
// DELETE (commit `3a45b1ab`), so a flagged `revoked_at` no longer produces any mint outcome to
// assert. The column's new inertness is pinned at the service layer by
// `a_flagged_revoked_at_no_longer_blocks_minting`; a standing refusal at the wire is covered by the
// end-to-end un-approved-link test.

/// A malformed principal is refused at the route, on the same shape checks link-state applies.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mint_rejects_a_malformed_principal(pool: PgPool) {
    let app = setup_slack_app(&pool).await;

    let res = post_mint(&app, "not-a-slack-principal", MINT_SECRET.as_bytes(), None).await;
    assert_eq!(
        res.status(),
        400,
        "a principal without the slack: prefix must be refused before any vault lookup",
    );
}

/// With `SLACK_MINT_SECRET` unset the mint endpoint is DISABLED — and the link flow still works.
///
/// Two assertions, and the second is the point. Fail-closed is easy to get right by accident (an
/// unset key trivially fails an HMAC comparison); what is easy to get WRONG is the blast radius.
/// Folding the mint key into `parse_slack_link`'s all-or-nothing set would have made an unset
/// mint secret silently disable the entire link flow — which is live in production — turning a
/// missing variable into an outage rather than one dark endpoint. This test is what would catch
/// that regression.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mint_is_disabled_without_its_secret_but_linking_still_works(pool: PgPool) {
    let app = setup_slack_app_with_mint_secret(&pool, None).await;
    link_a_vaulted_principal(&app, &pool).await;

    // Correctly signed with the key the ENABLED instance uses. It must still be refused, because
    // this instance has no mint key at all.
    let res = post_mint(&app, SLACK_PRINCIPAL, MINT_SECRET.as_bytes(), None).await;
    assert_eq!(
        res.status(),
        401,
        "an instance with no mint secret must refuse every mint, however well signed",
    );

    // The blast radius: link-state is untouched. An operator who has not yet set the mint secret
    // has a dark mint endpoint, NOT a broken Slack integration.
    let res = post_link_state(&app, SLACK_PRINCIPAL, None).await;
    assert_eq!(
        res.status(),
        200,
        "the link flow must survive an unset mint secret — the two keys are independent",
    );
}
