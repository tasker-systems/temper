//! The Slack account-link flow. Two endpoints, two audiences.
//!
//! `slack_link_state` is server-to-server (the mention agent, HMAC-gated). `callback` is
//! browser-facing and renders HTML, never JSON — a human is looking at it.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use jsonwebtoken::{decode, TokenData};

use temper_auth::{build_authorize_url, generate_pkce_pair, AuthorizeParams};
use temper_services::auth::AuthenticatedProfile;
use temper_services::auth::{AuthzError, RawJwtClaims};
use temper_services::error::ApiError;
use temper_services::services::{slack_grant_vault_service, slack_link_service};
use temper_services::state::AppState;
use temper_services::{link_provider, oauth_client};

/// How long a link URL stays usable. Long enough for a human to notice the ephemeral
/// message and finish a browser login; short enough to bound a stolen one.
const INTENT_TTL: Duration = Duration::from_secs(15 * 60);

/// The scopes the link grant requests. `offline_access` is what makes the exchange return a
/// refresh token — T2 obtains the grant; T3 vaults it.
///
/// `email` and `profile` are here because the email ladder is a HARD failure and runs before
/// resolution: with `openid` alone, Auth0's `/userinfo` returns no email, so the ladder's last
/// rung cannot succeed and every link attempt dies there. They match the Auth0-mode CLI
/// (`temper-cli/src/commands/init.rs`), and the temper AS advertises all four in
/// `scopes_supported` (`packages/temper-cloud/src/oauth/metadata.ts`), so this is safe in
/// both modes.
const LINK_SCOPES: [&str; 4] = ["openid", "profile", "email", "offline_access"];

/// Cap on `slack_principal_id`, matching `kb_profile_auth_links.auth_provider_user_id`'s
/// `VARCHAR(128)`. The intents table is TEXT, so without this a too-long principal mints an
/// intent, survives the exchange, BURNS the state, and only then fails at the final upsert —
/// the user sees a save error and has to start over. Reject at the door instead.
const MAX_SLACK_PRINCIPAL_LEN: usize = 128;

/// The prefix every eve Slack principal carries, whatever its segment count.
const SLACK_PRINCIPAL_PREFIX: &str = "slack:";

#[derive(Debug, serde::Deserialize)]
pub struct SlackLinkStateRequest {
    /// The WHOLE opaque principal from `attributes` — 2-4 segments, never split.
    pub slack_principal_id: String,
}

/// What the mention agent should say to this Slack user.
///
/// A discriminated union, not an `Option`-riddled struct: the two states carry disjoint data
/// (a linked user has a handle and NO authorize URL; an unlinked one the reverse), and a
/// struct with two nullable fields would make "both set" and "neither set" representable —
/// two states that must not exist. The agent mirrors this union in `agent/lib/link.ts`, so
/// both ends are forced to handle both arms.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SlackLinkStateResponse {
    /// A `kb_profile_auth_links` row already binds this principal. Nothing to mint.
    Linked {
        /// The profile's slug. The wire key is `handle` — the word a Slack user understands.
        handle: String,
    },
    /// No link row. This is the only arm that mints an intent.
    Unlinked { authorize_url: String },
}

/// `POST /internal/slack/link-state` — answer "what do I say to this Slack user?"
///
/// The agent's real question per mention is not "mint me a URL" — it is what to say. Asking
/// for a URL unconditionally is what made an already-linked user get re-prompted to link on
/// every single mention, forever, and minted a junk intent row each time. So the endpoint
/// answers the question: **the linked arm mints nothing**.
///
/// Gated by `require_slack_link_signature`. The signature covers THIS call, not the URL the
/// user later clicks: `internal_sig`'s skew window is 30s and a human clicks minutes later,
/// so signing the user-facing URL would force us to loosen a gate that is tight for good
/// reason. What the user receives is the IdP's own authorize URL with an opaque state.
pub async fn slack_link_state(
    State(state): State<AppState>,
    Json(req): Json<SlackLinkStateRequest>,
) -> Result<Json<SlackLinkStateResponse>, ApiError> {
    validate_slack_principal(&req.slack_principal_id)?;

    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;

    // The read comes FIRST and short-circuits: an already-linked principal never reaches the
    // mint below. That ordering is the whole fix.
    if let Some(handle) =
        slack_link_service::lookup_linked_handle(&state.pool, &req.slack_principal_id).await?
    {
        return Ok(Json(SlackLinkStateResponse::Linked { handle }));
    }

    let provider = link_provider::derive(&state.config.auth, cfg);

    let (verifier, challenge) = generate_pkce_pair();
    let state_nonce = slack_link_service::create_intent(
        &state.pool,
        &req.slack_principal_id,
        &verifier,
        INTENT_TTL,
    )
    .await?;

    let authorize_url = build_authorize_url(&AuthorizeParams {
        authorize_url: provider.authorize_url,
        client_id: provider.client_id,
        audience: Some(state.config.auth.audience.clone()),
        redirect_uri: provider.redirect_uri,
        scopes: LINK_SCOPES.iter().map(|s| (*s).to_string()).collect(),
        state: state_nonce,
        code_challenge: challenge,
    })
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SlackLinkStateResponse::Unlinked { authorize_url }))
}

/// Reject a malformed principal HERE, not at the final INSERT.
///
/// Three checks, all shape and none semantic: non-empty, within the storage column's width,
/// and carrying the `slack:` prefix. The principal is OPAQUE — 2 to 4 segments depending on
/// whether a team id is present and whether the author is a bot — so it is deliberately
/// NEVER split on ':'. A prefix check plus a length check is the whole of what is knowable
/// without parsing something we have no business parsing.
pub(crate) fn validate_slack_principal(principal: &str) -> Result<(), ApiError> {
    if principal.is_empty() {
        return Err(ApiError::BadRequest(
            "slack_principal_id must not be empty".to_string(),
        ));
    }
    if principal.len() > MAX_SLACK_PRINCIPAL_LEN {
        return Err(ApiError::BadRequest(format!(
            "slack_principal_id exceeds the maximum length of {MAX_SLACK_PRINCIPAL_LEN} bytes"
        )));
    }
    if !principal.starts_with(SLACK_PRINCIPAL_PREFIX) {
        return Err(ApiError::BadRequest(format!(
            "slack_principal_id must start with '{SLACK_PRINCIPAL_PREFIX}'"
        )));
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// `GET /api/auth/slack/callback` — the registered redirect_uri.
///
/// Renders HTML on every path. Never JSON, and never a redirect back to Slack: the human is
/// already looking at this page, so it IS the confirmation. temper-api holds no Slack
/// credential and knows no channel.
pub async fn callback(State(state): State<AppState>, Query(q): Query<CallbackQuery>) -> Response {
    match run_callback(&state, q).await {
        Ok(slug) => connected_page(&slug),
        Err(message) => not_connected_page(&message),
    }
}

/// The flow proper. Returns the linked profile's slug, or a user-actionable message.
///
/// `Profile` has NO `handle` field — it is `slug` (`temper-core/src/types/profile.rs:23`).
/// The `handle` on the team types is a different thing; do not reach for it.
///
/// Every `Err` string here is shown to a human, so none of them may reveal whether a given
/// profile exists.
async fn run_callback(state: &AppState, q: CallbackQuery) -> Result<String, String> {
    if let Some(err) = q.error {
        tracing::warn!(error = %err, "slack link: IdP returned an error");
        return Err("The sign-in was cancelled or refused. Mention @temper again to retry.".into());
    }

    let (Some(code), Some(state_nonce)) = (q.code, q.state) else {
        return Err("That link is incomplete. Mention @temper again to get a fresh one.".into());
    };

    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| "Account linking is not configured on this instance.".to_string())?;
    let provider = link_provider::derive(&state.config.auth, cfg);

    // Single-use + TTL + unguessability, in one atomic burn. Unknown, expired and replayed
    // are indistinguishable here BY DESIGN — do not try to tell the user which it was.
    let intent = slack_link_service::consume_intent(&state.pool, &state_nonce)
        .await
        .map_err(|_| "Something went wrong. Mention @temper again to retry.".to_string())?
        .ok_or_else(|| {
            tracing::warn!("slack link: rejected (unknown, expired or replayed state)");
            "That link has expired or was already used. Mention @temper again to get a fresh one."
                .to_string()
        })?;

    let tokens = oauth_client::exchange_code(
        &provider.token_url,
        &provider.client_id,
        &code,
        &intent.code_verifier,
        &provider.redirect_uri,
    )
    .await
    .map_err(|_| "Sign-in could not be completed. Mention @temper again to retry.".to_string())?;

    // LOOKUP-ONLY. Connecting Slack is not a registration route (spec D3).
    let profile = resolve_existing(state, &tokens.access_token)
        .await
        .map_err(|_| {
            "No temper account is linked to this login. Sign in at temperkb.io first, then \
             mention @temper again to connect."
                .to_string()
        })?;

    // ONE TRANSACTION for the identity row AND the sealed grant.
    //
    // They used to be two independent autocommits, and the window between them was not benign.
    // Process death after the directory row committed and before the vault row did left the user
    // LINKED with a refresh token that existed only in this stack frame — and unrecoverable,
    // because `consume_intent` burned the state nonce above, so no retry of this callback can
    // re-derive it. The old comment here argued the failure was benign because "the user is shown
    // a retry page"; that holds for an observable `Err` return below and does NOT hold for a kill
    // between two commits, where no page renders at all. A rollback of both is recoverable (the
    // user mentions @temper and gets a fresh intent); a half-write is not.
    let mut tx =
        state.pool.begin().await.map_err(|_| {
            "Something went wrong saving the link. Mention @temper again.".to_string()
        })?;

    // Auth before write: the profile is resolved and gated above this line.
    //
    // A principal binds ONCE. If it is already bound to a different profile the write is
    // refused rather than moved — see `link_slack_principal` and spec D4.
    let outcome = slack_link_service::link_slack_principal(
        &mut tx,
        profile.profile().id,
        &intent.slack_principal_id,
    )
    .await
    .map_err(|_| "Something went wrong saving the link. Mention @temper again.".to_string())?;

    if outcome == slack_link_service::SlackLinkOutcome::AlreadyLinkedToAnotherProfile {
        tracing::warn!(
            profile_id = %profile.profile().id,
            "slack link: refused (principal already bound to a different profile)",
        );
        // DELIBERATE, BOUNDED DISCLOSURE. This message admits the principal IS linked, which
        // the generic "no temper account is linked" refusal does not. That is the right
        // trade: the legitimate user needs to understand why their link failed and what to do
        // about it, and collapsing this into the generic message would actively mislead them
        // — they'd be told to sign in at temperkb.io, which would not help. An attacker who
        // stole a link URL learns only that their attack failed. The other profile's handle is
        // NOT named: which account holds it is not theirs to learn.
        return Err(
            "This Slack account is already connected to a different temper account. \
                    Disconnect it there first, then mention @temper again to reconnect."
                .into(),
        );
    }

    // T3 SEAM. The exchange requested `offline_access`, so `tokens.refresh_token` is the
    // per-user grant -- its own independent family, never an export of the user's CLI grant.
    // Encrypt and store it here, keyed by the whole opaque principal. Identity (the row above)
    // and secret (this vault) stay in separate tables; kb_profile_auth_links has no secret column.
    //
    // Auth-before-write: the profile is resolved and gated well above this line, and the vault
    // write comes after. Both now ride `tx`, so a failure on either rolls back BOTH — there is no
    // longer a "linked but not vaulted" state for this handler to leave behind.
    let Some(refresh_token) = tokens.refresh_token.as_deref() else {
        // `offline_access` was requested but the IdP returned no refresh token — a client
        // misconfiguration (offline_access or refresh-token rotation not enabled on the link
        // client). There is nothing to vault, so acting as the human is impossible: this link
        // could never mint.
        //
        // THIS ARM MUST NOT RENDER SUCCESS. It used to `warn!` and return `Ok(slug)`, which
        // rendered "Account connected." at a user whose link was inert — they were told the
        // thing worked, then hit an unexplained failure at their next mention, with no reason
        // to suspect the link. Telling them the truth costs one retry; the lie costs a silent
        // dead end. (`tests/e2e/tests/slack_link_test.rs` documented that broken shape.)
        //
        // `tx` is DROPPED without commit, so the directory row is rolled back too. That is the
        // deliberate call: half-linked is the worst of the three states. Linked-with-no-grant
        // reads as "connected" to `lookup_linked_handle` (which reads the auth-link table, not
        // the vault), so link-state would keep telling the agent this user is fine while every
        // mint answers `not_vaulted` — and re-linking cannot repair it, because
        // `link_slack_principal` refuses to rebind. Rolling back leaves the user cleanly
        // UNLINKED, which is the one state the flow can recover from on its own: they mention
        // @temper, get a fresh intent, and try again once the operator fixes the client.
        tracing::warn!(
            profile_id = %profile.profile().id,
            "slack link: NO refresh token returned -- link rolled back rather than left inert; \
             check the link client's offline_access / refresh-token-rotation settings",
        );
        return Err(
            "Your account was verified, but the connection could not be completed — this \
             instance's sign-in client did not return the credential needed to act on your \
             behalf. Nothing was saved. Mention @temper again to retry, and tell your temper \
             administrator if it keeps happening."
                .into(),
        );
    };

    slack_grant_vault_service::store_grant(
        &mut tx,
        &cfg.vault_key,
        slack_grant_vault_service::NewGrant {
            profile_id: profile.profile().id,
            slack_principal_id: &intent.slack_principal_id,
            refresh_token,
            access_token: &tokens.access_token,
            access_ttl_secs: tokens.expires_in,
        },
    )
    .await
    .map_err(|_| {
        "Something went wrong saving your connection. Mention @temper again.".to_string()
    })?;

    // The commit is the point: identity and grant land together or not at all.
    tx.commit().await.map_err(|_| {
        "Something went wrong saving your connection. Mention @temper again.".to_string()
    })?;

    tracing::info!(profile_id = %profile.profile().id, "slack link: established and grant vaulted");

    Ok(profile.profile().slug.clone())
}

/// Verify the freshly-exchanged access token and resolve the profile it names — **lookup-only**.
///
/// The token arrived over our own back-channel exchange, which is exactly the reasoning that
/// must not be relied on: a token we did not verify is a token we did not authenticate,
/// whatever channel delivered it. So this walks the identical JWKS path as
/// `middleware::auth::require_auth` — same key store, same algorithm-scoped validation, same
/// `decode` — before handing the verified claims to the seam.
///
/// `authenticate_token_existing_only`, never `authenticate_token`: the latter auto-provisions
/// a profile, which on a stray click would mint an account and confer auto-join team reach.
/// Linking an existing identity is not a registration route.
async fn resolve_existing(
    state: &AppState,
    access_token: &str,
) -> Result<AuthenticatedProfile, ApiError> {
    let vk = state.jwks_store.get_decoding_key().await.map_err(|e| {
        tracing::error!("slack link: JWKS key retrieval failed: {e}");
        ApiError::Unauthorized("Authentication service unavailable".to_string())
    })?;

    let issuer = &state.config.auth.issuer;
    let audience = state.config.auth.audience.as_str();
    let validation = state.jwks_store.validation(issuer, audience, vk.algorithm);

    let token_data: TokenData<RawJwtClaims> =
        decode(access_token, &vk.key, &validation).map_err(|e| {
            tracing::debug!("slack link: JWT verification failed: {e}");
            ApiError::Unauthorized("Invalid or expired token".to_string())
        })?;
    let raw = token_data.claims;

    temper_services::auth::authenticate_token_existing_only(state, &raw, access_token)
        .await
        .map_err(|e| {
            // Every arm collapses to ONE refusal, here and in the caller, so that no page
            // ever reveals whether a profile exists (D3). Mapping the arms to distinct
            // `ApiError`s would only look like it did something: `run_callback` discards
            // this value and renders a single fixed sentence. The seam has already logged
            // each reason with the `sub`.
            //
            // The one arm worth distinguishing is `Deactivated`, and what makes it worth it
            // is the log line, not the error — a deactivated profile reaching the link flow
            // is an operator-visible event.
            if let AuthzError::Deactivated { profile_id } = e {
                tracing::warn!(%profile_id, "slack link: rejected (profile is deactivated)");
            }
            ApiError::Unauthorized("Invalid or expired token".to_string())
        })
}

// ── Branded callback pages ──────────────────────────────────────────────────────
//
// The human is looking straight at this page, so it IS the confirmation — it must feel like
// temper, not a bare `<h1>`. The look is ported from the CLI-auth callback pages
// (`temper-client/src/login_page.rs`): obsidian ground, parchment serif, one steel-blue accent,
// the inline threaded-t wordmark. It is NOT imported: temper-api must not depend on
// temper-client (that inverts the server→client direction), and this is a few dozen lines of
// static HTML that rarely change — duplication is the right call over a manufactured dependency.
//
// DELIBERATELY self-contained with NO external asset requests — no font `<link>`, no images. The
// browser hitting this callback may have no route to temper's assets, and the connected/failed
// message must render regardless; the `font-family` fallbacks carry the look offline. (The CLI
// page keeps a Google Fonts `<link>`; here it is dropped on purpose.)

/// The threaded-t brand mark + wordmark, inlined. Geometry lifted from `login_page.rs`.
const BRAND_MARK: &str = r##"<svg width="118" height="24" viewBox="0 0 200 40" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
  <path d="M 12 6 L 12 34" stroke="#7eb8da" stroke-width="3.5" stroke-linecap="round" fill="none"/>
  <path d="M 4 16 L 20 16 Q 27 16 30 21 Q 33 26 31 32" stroke="#7eb8da" stroke-width="2.6" stroke-linecap="round" fill="none"/>
  <text x="46" y="27" font-family="'JetBrains Mono','Fira Code',monospace" font-size="18" fill="#7eb8da" letter-spacing="0.12em">temper</text>
</svg>"##;

/// Page skeleton. `{{MARK}}`/`{{EYEBROW}}`/`{{HEADING}}`/`{{BODY}}` are filled by [`render`];
/// placeholders (not `format!`) keep the CSS braces intact.
const TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>temper · Slack</title>
<style>
  :root {
    --obsidian: #0a0a0f;
    --parchment: #e8e4df;
    --chalk: rgba(255, 255, 255, 0.65);
    --temper-blue: #7eb8da;
    --serif: "Source Serif 4", "Source Serif Pro", Georgia, "Times New Roman", serif;
    --mono: "JetBrains Mono", "Fira Code", ui-monospace, monospace;
  }
  * { box-sizing: border-box; }
  html, body { height: 100%; }
  body {
    margin: 0;
    background: var(--obsidian);
    color: var(--parchment);
    font-family: var(--serif);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    -webkit-font-smoothing: antialiased;
    text-rendering: optimizeLegibility;
  }
  .card {
    width: 100%;
    max-width: 30rem;
    border-left: 2px solid rgba(126, 184, 218, 0.25);
    padding-left: 1.9rem;
  }
  .mark { margin-bottom: 1.7rem; line-height: 0; }
  .eyebrow {
    font-family: var(--mono);
    font-size: 0.65rem;
    letter-spacing: 0.2em;
    text-transform: uppercase;
    color: var(--temper-blue);
    margin-bottom: 0.9rem;
  }
  .heading {
    font-family: var(--serif);
    font-weight: 300;
    font-size: clamp(1.7rem, 5vw, 2rem);
    line-height: 1.25;
    margin: 0 0 1.05rem 0;
    color: var(--parchment);
  }
  .heading em { font-style: italic; color: var(--temper-blue); }
  .body {
    font-family: var(--serif);
    font-size: 1rem;
    line-height: 1.75;
    color: var(--chalk);
    margin: 0;
  }
  .body strong { color: var(--parchment); font-weight: 400; }
</style>
</head>
<body>
  <main class="card">
    <div class="mark">{{MARK}}</div>
    <div class="eyebrow">{{EYEBROW}}</div>
    <h1 class="heading">{{HEADING}}</h1>
    {{BODY}}
  </main>
</body>
</html>"##;

/// Assemble a callback page. `heading_html` carries its own static `<em>` accent and is trusted
/// markup; `body_html` is assembled by the callers below, which escape any dynamic text first.
fn render(eyebrow: &str, heading_html: &str, body_html: &str) -> Response {
    let html = TEMPLATE
        .replace("{{MARK}}", BRAND_MARK)
        .replace("{{EYEBROW}}", eyebrow)
        .replace("{{HEADING}}", heading_html)
        .replace("{{BODY}}", body_html);
    Html(html).into_response()
}

/// Shown when the account is now linked. `slug` is the resolved profile handle, escaped.
fn connected_page(slug: &str) -> Response {
    let body = format!(
        "<p class=\"body\">Linked as <strong>@{}</strong>. You can close this tab and return to \
         Slack.</p>",
        html_escape(slug)
    );
    render("temper · Slack", "Account <em>connected</em>.", &body)
}

/// Shown for every non-success outcome. `message` is one of `run_callback`'s user-actionable
/// sentences (already free of any secret) and is escaped before display.
fn not_connected_page(message: &str) -> Response {
    let body = format!("<p class=\"body\">{}</p>", html_escape(message));
    render("temper · Slack", "Not <em>connected</em>.", &body)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All four principal shapes eve emits (2-4 segments) pass. If any of these were
    /// rejected the guard would be parsing, which is exactly what it must not do.
    #[test]
    fn accepts_every_shape_of_real_principal() {
        for p in [
            "slack:T123:U456",     // team + human
            "slack:T123:bot:U456", // team + bot
            "slack:U456",          // no team + human
            "slack:bot:U456",      // no team + bot
        ] {
            assert!(validate_slack_principal(p).is_ok(), "rejected {p}");
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_slack_principal("").is_err());
    }

    #[test]
    fn rejects_a_principal_wider_than_the_storage_column() {
        // 128 fits; 129 does not — the boundary is the VARCHAR(128) that would otherwise
        // fail at the upsert, after the state was already burned.
        let at_limit = format!("slack:{}", "u".repeat(MAX_SLACK_PRINCIPAL_LEN - 6));
        assert_eq!(at_limit.len(), MAX_SLACK_PRINCIPAL_LEN);
        assert!(validate_slack_principal(&at_limit).is_ok());

        let over = format!("slack:{}", "u".repeat(MAX_SLACK_PRINCIPAL_LEN));
        assert!(validate_slack_principal(&over).is_err());
    }

    #[test]
    fn rejects_a_foreign_prefix() {
        assert!(validate_slack_principal("discord:T123:U456").is_err());
        assert!(validate_slack_principal("U456").is_err());
    }

    /// Extract a rendered page's HTML body.
    async fn body_html(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// The rendered page carries the temper look: obsidian ground, the steel-blue accent, and the
    /// inline threaded-t wordmark — matching `login_page.rs`.
    #[tokio::test]
    async fn connected_page_carries_the_brand_and_the_handle() {
        let html = body_html(connected_page("j-cole-taylor")).await;
        assert!(html.contains("Account <em>connected</em>."));
        assert!(html.contains("@j-cole-taylor"));
        assert!(html.contains("#0a0a0f"), "obsidian ground present");
        assert!(html.contains("temper"), "wordmark present");
    }

    /// The fold-in's load-bearing constraint: the page fetches NOTHING to render. No font link,
    /// no remote stylesheet, no image — the callback browser may have no route to temper's assets.
    ///
    /// Checks the actual fetch vectors, not any `http` substring: the inline SVG legitimately
    /// carries the `xmlns="http://www.w3.org/2000/svg"` NAMESPACE, which is declarative and never
    /// fetched. What must be absent is anything that triggers a network load.
    #[tokio::test]
    async fn pages_request_no_external_assets() {
        for html in [
            body_html(connected_page("someone")).await,
            body_html(not_connected_page("That link has expired.")).await,
        ] {
            assert!(
                !html.contains("<link"),
                "no external <link> stylesheet/font"
            );
            assert!(!html.contains("googleapis"), "no web-font fetch");
            assert!(!html.contains("@import"), "no CSS @import");
            assert!(!html.contains("src="), "no <img>/<script> src");
            assert!(!html.contains("href=\"http"), "no external href");
            assert!(!html.contains("url(http"), "no CSS url() fetch");
        }
    }

    /// A dynamic message is HTML-escaped — a refusal sentence can never break out of the page.
    #[tokio::test]
    async fn not_connected_page_escapes_the_message() {
        let html = body_html(not_connected_page("<script>alert(1)</script>")).await;
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
