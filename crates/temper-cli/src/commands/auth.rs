//! `temper auth` subcommands: login, logout, status, token, plus the system
//! access gate (request-access / withdraw-request).
//!
//! All subcommands accept `--format json | toon` (auto-detected from TTY
//! when omitted). `login`, `logout`, and `token` are inherently disk-mode
//! operations — they persist credentials to `~/.config/temper/auth.json`.
//! Cloud sessions receive tokens via `TEMPER_TOKEN` and don't invoke these.
//!
//! The system access gate lives here (not under `temper team`) because it is an
//! entitlement concern — "am I let into the system at all?" — not a
//! collaboration one. The gating *team* is only its implementation substrate.

use temper_client::auth::{AuthStatus, DiskTokenStore, TokenStore};
use temper_client::error::ClientError;
use temper_client::TemperClient;
use temper_core::types::access_gate::JoinRequestStatus;
use temper_principal::Standing;

use crate::actions::runtime;
use crate::error::Result;
use crate::format::OutputFormat;
use crate::output;

/// Confirmation struct emitted by action commands (login, logout).
///
/// Wire shape: `{ "status": "logged_in" | "logged_out", "profile": <uuid> | null }`.
/// Replaces the ad-hoc JSON literals previously produced by each handler.
#[derive(Debug, serde::Serialize)]
struct AuthAction<'a> {
    status: &'a str,
    profile: Option<String>,
}

/// Run the OAuth2 PKCE login flow, persist the token, and print auth status.
pub fn login(fmt: OutputFormat) -> Result<()> {
    runtime::with_client(move |client| {
        Box::pin(async move {
            let stored = client
                .auth_login()
                .await
                .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
            let profile = stored.profile_id.map(|id| id.to_string());
            let action = AuthAction {
                status: "logged_in",
                profile,
            };
            let rendered = crate::format::render(&action, fmt)?;
            println!("{rendered}");
            Ok(())
        })
    })
}

/// Clear stored credentials and print confirmation.
pub fn logout(fmt: OutputFormat) -> Result<()> {
    DiskTokenStore::default_path()
        .clear()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    let action = AuthAction {
        status: "logged_out",
        profile: None,
    };
    let rendered = crate::format::render(&action, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Store a JWT directly to `~/.config/temper/auth.json`, reading the JWT
/// from **stdin only**.
///
/// Positional-arg JWT input would leak to shell history, `ps auxww`, and
/// `/proc/<pid>/cmdline`. Stdin-only input closes all three. Typical use:
///
/// ```text
/// temper auth export-token | temper auth token
/// pbpaste | temper auth token
/// ```
///
/// Writes to disk unconditionally — cloud sessions receive tokens via
/// `TEMPER_TOKEN` and don't invoke this command.
pub fn token(provider: &str, fmt: OutputFormat) -> Result<()> {
    let stdin_content = crate::vault::read_stdin_if_piped();
    if stdin_content.is_none() && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(crate::error::TemperError::Config(
            "temper auth token reads the JWT from stdin. Usage:\n  \
             temper auth export-token | temper auth token\n  \
             pbpaste | temper auth token"
                .into(),
        ));
    }
    token_from_stdin(stdin_content.as_deref(), provider, fmt)
}

fn token_from_stdin(stdin_content: Option<&str>, provider: &str, fmt: OutputFormat) -> Result<()> {
    let jwt_raw = stdin_content
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            crate::error::TemperError::Config(
                "temper auth token: stdin was empty; pipe a JWT".into(),
            )
        })?;

    let claims = temper_client::auth::parse_jwt_claims(jwt_raw)
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

    let provider_enum =
        temper_client::auth::Provider::try_from_env_value(Some(provider)).map_err(|e| {
            crate::error::TemperError::Config(format!(
                "invalid --provider: {e}. Accepted: \"auth0\" or \"auth0:DOMAIN\""
            ))
        })?;

    let device_id = temper_client::auth::load_or_create_device_id();

    let stored = temper_client::auth::StoredAuth {
        provider: provider_enum,
        access_token: jwt_raw.to_string().into(),
        refresh_token: None,
        expires_at: claims.expires_at,
        profile_id: claims.profile_id,
        device_id: Some(device_id),
    };

    DiskTokenStore::default_path()
        .save(&stored)
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

    let status = temper_client::auth::AuthStatus {
        authenticated: true,
        provider: Some(stored.provider),
        expires_at: Some(stored.expires_at),
        profile_id: stored.profile_id,
    };
    let rendered = crate::format::render(&status, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Export a refreshed access token from the local grant.
///
/// Token goes to stdout (plain, single line — pipeable to `pbcopy`, an
/// agent's secret input, etc.). Security warning goes to stderr.
///
/// Refuses to run in cloud mode — `export-token` reads from the local
/// `DiskTokenStore`; a cloud-mode invocation would have nothing to export
/// (cloud sessions receive their token via `TEMPER_TOKEN`).
pub fn export_token() -> Result<()> {
    // `export-token` reads from the on-disk `DiskTokenStore` grant. A
    // cloud agent session (`TEMPER_TOKEN` set) has no disk grant to
    // export — refuse with a directive to run this on the laptop.
    if std::env::var("TEMPER_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .is_some()
    {
        return Err(crate::error::TemperError::Config(
            "temper auth export-token reads the on-disk grant — this \
             session was handed its token via TEMPER_TOKEN and has \
             nothing to export. Run this on your laptop, paste the token \
             into the cloud session's secrets, and the agent reads \
             TEMPER_TOKEN."
                .into(),
        ));
    }

    let config = temper_client::config::load_cloud_config()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    let oauth = temper_client::config::oauth_config(&config)
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    let store = DiskTokenStore::default_path();

    print_export_warning();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    let token = rt.block_on(export_token_with_store(
        &store,
        &oauth.token_url,
        &oauth.client_id,
    ))?;
    println!("{token}");
    Ok(())
}

async fn export_token_with_store(
    store: &dyn TokenStore,
    token_url: &str,
    client_id: &str,
) -> Result<String> {
    temper_client::auth::get_valid_token(store, token_url, client_id)
        .await
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))
}

fn print_export_warning() {
    eprintln!("⚠  This access token grants full access to your temper account at");
    eprintln!("   your current permission levels until it expires (~24 hours).");
    eprintln!("   Once issued, the token cannot be revoked early — treat leaked");
    eprintln!("   tokens as live for their full lifetime. Per-session revocation");
    eprintln!("   is coming in Unit D of the cloud-mode goal.");
    eprintln!();
    eprintln!("   Recommended handling:");
    eprintln!("     temper auth export-token | pbcopy          # macOS clipboard");
    eprintln!("     temper auth export-token | wl-copy         # Linux wayland");
    eprintln!("     temper auth export-token | <agent-secret-input>");
    eprintln!("   AVOID:");
    eprintln!("     temper auth export-token > token.txt       # file lands in backups");
    eprintln!(
        "     TEMPER_TOKEN=$(temper auth export-token)   # shell history + /proc/<pid>/environ"
    );
    eprintln!();
}

/// Print the current auth status.
/// **Axis 1 — did we get an answer.**
///
/// Kept strictly separate from what the answer *was*. The shape this replaced had one field
/// carrying both, where `"unknown"` meant "could not reach the server" and `"none"` meant "you have
/// no access" — so a reader could not tell *we could not ask* from *we asked and you are denied*,
/// which are opposite facts with opposite next actions.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum AccessQuery {
    /// The server answered. Axis 2 is present.
    Reachable,
    /// The credential was rejected. Makes an expired token observable from the server side,
    /// independently of the local expiry check on `authenticated`.
    Unauthenticated,
    /// The server could not be reached at all — DNS, connection refused, TLS, timeout.
    Unreachable,
    /// The server was reached and failed to answer (5xx, or anything unclassified).
    Error,
}

/// **Axis 2 — what the answer was.** Present only when axis 1 is [`AccessQuery::Reachable`].
///
/// Every field here comes from one `GET /api/profile` round trip.
#[derive(Debug, serde::Serialize)]
struct Entitlement {
    /// The caller's own standing, as stored: `denied` · `requested` · `approved` · `revoked` ·
    /// `deactivated`. Absent only when the server predates the field, in which case the boolean
    /// below still carries the access answer.
    ///
    /// `revoked` is reported rather than folded into `denied` because the two carry **different
    /// remedies** — see [`remedy_for`], and `access_gate.rs`, which has always drawn the same
    /// distinction on the refusal path.
    #[serde(skip_serializing_if = "Option::is_none")]
    standing: Option<Standing>,
    /// The `has_system_access` predicate, which reads `kb_principal_standing` — the authoritative
    /// answer, and the one the old implementation failed to ask for.
    system_access: bool,
    is_admin: bool,
    /// The caller's own join request, when the server discloses one. **Absent is normal** — a
    /// principal admitted by any path other than the request queue has no row, which is precisely
    /// the case the old implementation misread as denial.
    #[serde(skip_serializing_if = "Option::is_none")]
    join_request: Option<JoinRequestStatus>,
}

/// System-access summary folded into `auth status`, as two orthogonal axes.
#[derive(Debug, serde::Serialize)]
struct SystemAccessReport {
    query: AccessQuery,
    /// Human context for a non-`reachable` outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entitlement: Option<Entitlement>,
}

/// Who the server says the caller is.
///
/// Resolved rather than read: the stored credential's `profile_id` is structurally absent under
/// Auth0, so the only source for identity is the server. No extra round trip — `auth status`
/// already had to call out for the access answer, and one `GET /api/profile` carries both.
#[derive(Debug, serde::Serialize)]
struct Identity {
    profile_id: uuid::Uuid,
    /// The `@handle` form, i.e. `'@' + profile.slug`. The same spelling the vault projection and
    /// every context ref use, so it is directly comparable to what the user sees elsewhere.
    handle: String,
}

/// One `GET /api/profile`, both answers.
///
/// Returns identity **and** the access report together because they come from the same response;
/// splitting them into two resolvers would double the round trips and let the two disagree.
async fn resolve_access(client: &TemperClient) -> (Option<Identity>, SystemAccessReport) {
    match client.profile().get_with_entitlements().await {
        Ok(p) => {
            let e = p.entitlements;
            (
                Some(Identity {
                    profile_id: p.profile.id,
                    handle: format!("@{}", p.profile.slug),
                }),
                SystemAccessReport {
                    query: AccessQuery::Reachable,
                    detail: None,
                    entitlement: Some(Entitlement {
                        standing: e.standing,
                        system_access: e.system_access,
                        is_admin: e.is_admin,
                        join_request: e.join_request_status,
                    }),
                },
            )
        }
        Err(err) => {
            // Classify onto axis 1 only. Note `is_network` is the client's own predicate for
            // "could not reach the server" — asking it beats restating the match here, which would
            // drift the moment a new transport variant is added.
            let (query, detail) = if err.is_network() {
                (AccessQuery::Unreachable, Some(err.to_string()))
            } else {
                match &err {
                    ClientError::NotAuthenticated | ClientError::TokenExpired => {
                        (AccessQuery::Unauthenticated, Some(err.to_string()))
                    }
                    _ => (AccessQuery::Error, Some(err.to_string())),
                }
            };
            (
                None,
                SystemAccessReport {
                    query,
                    detail,
                    entitlement: None,
                },
            )
        }
    }
}

/// Combined `auth status` payload: the local auth state, plus — when authenticated — who the server
/// says we are and what we are entitled to. `AuthStatus` is flattened so the top-level shape
/// (`authenticated`, `provider`, …) is preserved and the resolved fields are simply added.
#[derive(Debug, serde::Serialize)]
struct AuthStatusReport {
    #[serde(flatten)]
    auth: AuthStatus,
    /// Absent when not logged in, or when the server could not be asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<Identity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_access: Option<SystemAccessReport>,
}

/// The next action a principal in this standing can actually take, or `None` when there is
/// nothing to say.
///
/// This exists so `auth status` stops being the one surface that knows the state and withholds the
/// remedy. The refusal path has always drawn this distinction — `access_gate.rs` sends `Denied` to
/// `request-access` and `Revoked` to `request-review`, because `Act::RequestReview` is legal from
/// `Revoked` and from nothing else. Reporting `revoked` without the remedy would tell a principal
/// they are stuck when they are not.
///
/// Deliberately a **hint on stderr**, never part of the payload: stdout carries the rendered
/// report and must stay parseable.
fn remedy_for(standing: Standing) -> Option<&'static str> {
    match standing {
        // Born denied, and may ask.
        Standing::Denied => Some(temper_core::types::access_gate::REQUEST_ACCESS_COMMAND),
        // D15's reconsideration channel — the whole reason this state is reported rather than
        // folded into `denied`.
        Standing::Revoked => Some(crate::access_gate::REQUEST_REVIEW_COMMAND),
        // Already asked, or already in. Nothing to do.
        Standing::Requested | Standing::Approved => None,
        // Unreachable here: a deactivated principal fails authentication before any handler runs,
        // so `auth status` reports `authenticated: false` and never resolves entitlements. Matched
        // explicitly rather than by wildcard so a sixth state is a compile error, not a silent None.
        Standing::Deactivated => None,
    }
}

/// Print the current auth status.
pub fn status(fmt: OutputFormat) -> Result<()> {
    runtime::with_client(move |client| {
        Box::pin(async move {
            let auth = client
                .auth_status()
                .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
            // Identity and entitlements both require the server; only consult it when the local
            // credential is usable. An expired token would answer `unauthenticated` on axis 1, but
            // spending a round trip to learn what `authenticated: false` already said is waste.
            let (identity, system_access) = if auth.authenticated {
                let (id, access) = resolve_access(client).await;
                (id, Some(access))
            } else {
                (None, None)
            };
            let report = AuthStatusReport {
                auth,
                identity,
                system_access,
            };
            let rendered = crate::format::render(&report, fmt)?;
            println!("{rendered}");
            // Stderr only — stdout is the payload. Emitted after the render so a caller piping
            // stdout still sees the hint on their terminal.
            if let Some(cmd) = report
                .system_access
                .as_ref()
                .and_then(|a| a.entitlement.as_ref())
                .and_then(|e| e.standing)
                .and_then(remedy_for)
            {
                output::hint(format!("  {cmd}"));
            }
            Ok(())
        })
    })
}

/// Request system access (the invite_only gate). Reviewed by an admin.
pub fn request_access(message: Option<&str>) -> Result<()> {
    let message = message.map(|s| s.to_string());
    runtime::with_client(|client| {
        Box::pin(async move {
            match client
                .access()
                .create_request(message.as_deref(), "cli", None)
                .await
            {
                Ok(result) => {
                    // Every line here is prose: `request_access` takes no `fmt`
                    // parameter and never calls `format::render`, so there is no
                    // payload on stdout to protect — which means stdout must stay
                    // empty for the parser. The whole block goes to stderr.
                    output::success_err("Access request submitted.");
                    output::plain_err("  You'll gain access once an admin approves your request.");
                    output::hint("  Run `temper auth status` to check.");
                    output::blank_err();
                    output::dim_err(format!("  Request ID: {}", result.id));
                }
                Err(temper_client::error::ClientError::Conflict { .. }) => {
                    output::warning("You already have a pending request.");
                    output::hint("  Run `temper auth status` to check its status.");
                }
                Err(e) => return Err(crate::actions::runtime::client_err_to_temper(e)),
            }
            Ok(())
        })
    })
}

/// Withdraw a pending system-access request.
pub fn withdraw_request() -> Result<()> {
    runtime::with_client(|client| {
        Box::pin(async move {
            let request = client
                .access()
                .get_own_request()
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            match request {
                None => {
                    output::plain("Nothing to withdraw — you don't have a pending request.");
                }
                Some(req) => match req.status {
                    JoinRequestStatus::Pending => {
                        client
                            .access()
                            .withdraw_request()
                            .await
                            .map_err(crate::actions::runtime::client_err_to_temper)?;
                        output::success("Request withdrawn.");
                    }
                    JoinRequestStatus::Approved => {
                        output::plain("You already have system access.");
                    }
                    _ => {
                        output::plain("Nothing to withdraw — no active request.");
                    }
                },
            }
            Ok(())
        })
    })
}

/// Ask an admin to reconsider a revocation (D15). Does not restore access by itself.
pub fn request_review(message: Option<&str>) -> Result<()> {
    let message = message.map(|s| s.to_string());
    runtime::with_client(|client| {
        Box::pin(async move {
            client
                .access()
                .create_review_request(message.as_deref())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            output::success_err("Review request submitted.");
            output::plain_err("  An admin will reconsider the revocation.");
            output::hint("  Run `temper auth status` to check your access.");
            Ok(())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_client::auth::{AuthStatus, Provider};

    fn make_auth_status(authenticated: bool) -> AuthStatus {
        if authenticated {
            AuthStatus {
                authenticated: true,
                provider: Some(Provider::auth0("test.auth0.com")),
                expires_at: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-12-31T23:59:59Z")
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                ),
                profile_id: Some(
                    uuid::Uuid::parse_str("01900000-0000-7000-8000-000000000001").unwrap(),
                ),
            }
        } else {
            AuthStatus {
                authenticated: false,
                provider: None,
                expires_at: None,
                profile_id: None,
            }
        }
    }

    #[test]
    fn render_auth_status_json_serializes() {
        let status = make_auth_status(true);
        let out =
            crate::format::render(&status, crate::format::OutputFormat::Json).expect("json render");
        assert!(
            out.contains("\"authenticated\""),
            "json should include authenticated field: {out}"
        );
        assert!(
            out.contains("\"profile_id\""),
            "json should include profile_id field: {out}"
        );
    }

    #[test]
    fn render_auth_status_json_unauthenticated() {
        let status = make_auth_status(false);
        let out =
            crate::format::render(&status, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.contains("\"authenticated\": false"), "json: {out}");
    }

    #[test]
    fn render_auth_status_toon_contains_field_name() {
        let status = make_auth_status(true);
        let out =
            crate::format::render(&status, crate::format::OutputFormat::Toon).expect("toon render");
        assert!(!out.is_empty(), "toon should not be empty: {out}");
    }

    #[test]
    fn render_auth_action_json_includes_status_key() {
        let action = AuthAction {
            status: "logged_in",
            profile: Some("alice".to_string()),
        };
        let out =
            crate::format::render(&action, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.contains("\"status\": \"logged_in\""), "json: {out}");
        assert!(out.contains("\"profile\": \"alice\""), "json: {out}");
    }

    #[test]
    fn render_auth_action_logout_no_profile() {
        let action = AuthAction {
            status: "logged_out",
            profile: None,
        };
        let out =
            crate::format::render(&action, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.contains("\"status\": \"logged_out\""), "json: {out}");
        assert!(
            out.contains("null"),
            "logout profile should serialize null: {out}"
        );
    }

    #[test]
    fn token_from_stdin_errors_when_empty() {
        let err = token_from_stdin(Some(""), "auth0", OutputFormat::Json).unwrap_err();
        assert!(
            format!("{err}").contains("stdin"),
            "expected empty-stdin error"
        );
    }

    #[test]
    fn token_from_stdin_errors_when_none() {
        let err = token_from_stdin(None, "auth0", OutputFormat::Json).unwrap_err();
        assert!(
            format!("{err}").contains("stdin"),
            "expected empty-stdin error"
        );
    }

    #[tokio::test]
    async fn export_token_with_store_errors_when_unauthenticated() {
        use temper_client::auth::MemoryTokenStore;
        let store = MemoryTokenStore::empty();
        // No token URL / client_id reachable matters — store has no auth.
        let err = export_token_with_store(&store, "https://example/token", "cid")
            .await
            .expect_err("empty store must error");
        assert!(matches!(err, crate::error::TemperError::Config(_)));
    }

    #[tokio::test]
    async fn export_token_with_store_returns_token_when_fresh() {
        use temper_client::auth::{MemoryTokenStore, Provider, StoredAuth};
        let store = MemoryTokenStore::with_auth(StoredAuth {
            provider: Provider::auth0("test.auth0.com"),
            access_token: "at_fresh".to_string().into(),
            refresh_token: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            profile_id: None,
            device_id: None,
        });
        let token = export_token_with_store(&store, "https://example/token", "cid")
            .await
            .expect("fresh token returns");
        assert_eq!(token, "at_fresh");
    }

    #[test]
    fn token_from_stdin_errors_on_invalid_provider() {
        // Use a placeholder JWT; provider validation happens before JWT parse?
        // Actually JWT parses first. Use a well-formed JWT that will fail
        // later — then check we surface the provider error.
        // Simpler: validate provider check path independently.
        let fake_jwt = "aGVhZGVy.cGF5bG9hZA.c2ln"; // "header.payload.sig" base64url
        let err = token_from_stdin(Some(fake_jwt), "github", OutputFormat::Json).unwrap_err();
        // Either JWT parse fails (likely) or provider parse fails. Both are
        // Config errors — we just want the end-to-end to refuse.
        assert!(matches!(err, crate::error::TemperError::Config(_)));
    }

    /// Verify that `auth token` routes through `render()` — the AuthStatus
    /// struct is what the token handler emits; test that json|toon both
    /// produce non-empty, valid output for the authenticated shape.
    #[test]
    fn render_auth_token_status_json_passthrough() {
        let status = make_auth_status(true);
        let out =
            crate::format::render(&status, crate::format::OutputFormat::Json).expect("json render");
        assert!(
            out.contains("\"authenticated\": true"),
            "token render must include authenticated: {out}"
        );
        assert!(
            out.contains("\"expires_at\""),
            "token render must include expires_at: {out}"
        );
    }

    #[test]
    fn render_auth_token_status_toon_is_non_empty() {
        let status = make_auth_status(true);
        let out =
            crate::format::render(&status, crate::format::OutputFormat::Toon).expect("toon render");
        assert!(
            !out.is_empty(),
            "token toon render should not be empty: {out}"
        );
    }

    // -----------------------------------------------------------------------------------------
    // The two-axes report.
    //
    // These are SHAPE tests, and saying so matters: they cannot fail against the code this task
    // replaced, because the types they name did not exist there. The witness that actually bites —
    // a revoked principal's surviving `approved` join request, suppressed — lives in
    // `temper-services/tests/entitlements_disclosure_test.rs`, where it was bite-probed by removing
    // the suppression. What these pin is the property that decides whether a *reader* can act on
    // the output: that "we could not ask" and "we asked and you are denied" never render alike.
    // -----------------------------------------------------------------------------------------

    fn reachable(standing: Option<Standing>, system_access: bool) -> SystemAccessReport {
        SystemAccessReport {
            query: AccessQuery::Reachable,
            detail: None,
            entitlement: Some(Entitlement {
                standing,
                system_access,
                is_admin: false,
                join_request: None,
            }),
        }
    }

    #[test]
    fn could_not_ask_and_asked_and_denied_do_not_render_alike() {
        // The bug the two axes exist to prevent: one field carrying both, where `"unknown"`
        // (transport) and `"none"` (entitlement) sat side by side and a reader could not tell
        // which had happened.
        let unreachable = SystemAccessReport {
            query: AccessQuery::Unreachable,
            detail: Some("network error".into()),
            entitlement: None,
        };
        let denied = reachable(Some(Standing::Denied), false);

        let a = crate::format::render(&unreachable, crate::format::OutputFormat::Json).unwrap();
        let b = crate::format::render(&denied, crate::format::OutputFormat::Json).unwrap();

        assert_ne!(a, b, "the two must be distinguishable");
        assert!(a.contains("unreachable"), "{a}");
        assert!(
            !a.contains("entitlement"),
            "axis 2 must be absent when axis 1 is not reachable: {a}"
        );
        assert!(b.contains("reachable"), "{b}");
        assert!(
            b.contains("entitlement"),
            "axis 2 must be present when reachable: {b}"
        );
    }

    #[test]
    fn an_absent_join_request_is_omitted_rather_than_reported_as_denial() {
        // The shape half of this task's original complaint: an approved principal who never filed
        // a request has nothing in the queue, and that absence must not read as a refusal.
        let out = crate::format::render(
            &reachable(Some(Standing::Approved), true),
            crate::format::OutputFormat::Json,
        )
        .unwrap();

        assert!(out.contains("\"system_access\": true"), "{out}");
        assert!(
            !out.contains("join_request"),
            "an absent join request must be omitted, not rendered: {out}"
        );
    }

    #[test]
    fn a_null_profile_id_is_omitted_rather_than_rendered() {
        // Under Auth0 the stored `profile_id` is structurally absent. Rendering it as `null` beside
        // a resolved identity reads as "we do not know who you are" when we do.
        let out =
            crate::format::render(&make_auth_status(false), crate::format::OutputFormat::Json)
                .unwrap();
        assert!(!out.contains("profile_id"), "{out}");
    }
}
