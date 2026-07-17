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

use temper_api::create_app;
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::{ApiConfig, SlackLinkConfig};
use temper_services::state::{AppState, JwksKeyStore};

/// The WHOLE opaque principal. Four segments' worth of Slack ids joined by colons —
/// carried verbatim, never split (spec: the segment count is not ours to assume).
const SLACK_PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

/// Shared with the "mention agent" (this test, standing in for it). Gates the intent route.
const SLACK_SECRET: &str = "slack-link-e2e-secret";

const CLIENT_ID: &str = "slack-link-client";

/// Matches `ApiConfig.auth_provider_name` below. The lookup-only resolve keys on
/// `(auth_provider, external_user_id)` using the SERVER's configured provider name, so an
/// "existing profile" means a `kb_profile_auth_links` row under exactly this string.
const PROVIDER: &str = "test-provider";

/// The authorization code the stub IdP accepts. Opaque to everything under test.
const AUTH_CODE: &str = "test-authorization-code";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A running API server whose configured issuer IS `idp` — see the module doc.
struct SlackLinkApp {
    addr: std::net::SocketAddr,
    http: reqwest::Client,
    /// Kept alive for the test's duration: dropping it stops serving the token endpoint.
    idp: MockServer,
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
}

/// Spawn the API with the Slack link fully configured and the issuer pointed at a stub IdP.
///
/// Mirrors `common::setup`'s server half (same RSA fixture, same static key store, same
/// `create_app`); it does not reuse it because `setup` hard-codes `issuer: "test-issuer"`
/// and `slack_link: None`, which are the two values this flow turns on.
async fn setup_slack_app(pool: &PgPool) -> SlackLinkApp {
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
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: Some(SlackLinkConfig {
            client_id: CLIENT_ID.to_string(),
            hmac_secret: SLACK_SECRET.to_string(),
            public_base_url: "https://temper.test".to_string(),
        }),
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

#[derive(Debug, Serialize)]
struct StubTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

/// Mount the RFC 6749 token endpoint on the stub, returning `access_token` on any exchange.
async fn stub_token_endpoint(app: &SlackLinkApp, access_token: String) {
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(StubTokenResponse {
            access_token,
            refresh_token: "stub-refresh-token".to_string(),
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

/// Mint an intent and return the opaque `state` the IdP would echo back to the callback.
async fn mint_state_nonce(app: &SlackLinkApp) -> String {
    let res = post_link_state(app, SLACK_PRINCIPAL, None).await;
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
/// `GET /api/profile`, exactly as a first sign-in does.
///
/// Deliberately not a hand-written INSERT: the row this flow must find is the row the login
/// path writes, and only the login path knows its full shape (profile + auth link under
/// `PROVIDER` + the surface emitters). A fixture INSERT would let the two drift.
async fn provision_profile(app: &SlackLinkApp, sub: &str, email: &str) {
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
