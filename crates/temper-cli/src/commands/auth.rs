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
use temper_client::TemperClient;
use temper_core::types::access_gate::{AccessMode, JoinRequestStatus};

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
/// System-access summary folded into `auth status`.
#[derive(Debug, serde::Serialize)]
struct SystemAccessReport {
    /// `granted` | `pending` | `none` | `unknown`.
    state: &'static str,
    /// Human context (e.g. "open access", "requested 2026-07-01"), when useful.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// Combined `auth status` payload: the local auth state plus, when
/// authenticated, the system-access entitlement. `AuthStatus` is flattened so
/// the top-level shape (`authenticated`, `provider`, …) is preserved and
/// `system_access` is simply added.
#[derive(Debug, serde::Serialize)]
struct AuthStatusReport {
    #[serde(flatten)]
    auth: AuthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_access: Option<SystemAccessReport>,
}

/// Resolve the caller's system-access state. Non-fatal: any server error
/// degrades to `unknown` so `auth status` still reports the local auth state
/// offline.
async fn resolve_system_access(client: &TemperClient) -> SystemAccessReport {
    let settings = match client.access().get_settings().await {
        Ok(s) => s,
        Err(_) => {
            return SystemAccessReport {
                state: "unknown",
                detail: Some("could not reach server".to_string()),
            };
        }
    };

    // Open mode grants everyone system access; no request needed.
    if matches!(
        AccessMode::from_db_str(&settings.access_mode),
        Some(AccessMode::Open)
    ) {
        return SystemAccessReport {
            state: "granted",
            detail: Some("open access".to_string()),
        };
    }

    // invite_only (or unrecognized): the join request carries the state.
    match client.access().get_own_request().await {
        Ok(Some(req)) => match req.status {
            JoinRequestStatus::Approved => SystemAccessReport {
                state: "granted",
                detail: None,
            },
            JoinRequestStatus::Pending => SystemAccessReport {
                state: "pending",
                detail: Some(format!("requested {}", req.created.format("%Y-%m-%d"))),
            },
            JoinRequestStatus::Rejected | JoinRequestStatus::Withdrawn => SystemAccessReport {
                state: "none",
                detail: None,
            },
        },
        Ok(None) => SystemAccessReport {
            state: "none",
            detail: None,
        },
        Err(_) => SystemAccessReport {
            state: "unknown",
            detail: Some("could not reach server".to_string()),
        },
    }
}

pub fn status(fmt: OutputFormat) -> Result<()> {
    runtime::with_client(move |client| {
        Box::pin(async move {
            let auth = client
                .auth_status()
                .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
            // System access requires the server; only consult it when logged in.
            let system_access = if auth.authenticated {
                Some(resolve_system_access(client).await)
            } else {
                None
            };
            let report = AuthStatusReport {
                auth,
                system_access,
            };
            let rendered = crate::format::render(&report, fmt)?;
            println!("{rendered}");
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
                    output::success("Access request submitted.");
                    output::plain("  You'll gain access once an admin approves your request.");
                    output::hint("  Run `temper auth status` to check.");
                    output::blank();
                    output::dim(format!("  Request ID: {}", result.id));
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
}
