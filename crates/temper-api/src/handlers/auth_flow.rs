//! CLI auth flow handlers — login initiation and callback.
//!
//! These are public (no JWT required) and handle the browser-based
//! OAuth flow for CLI authentication via Neon Auth (Better Auth).

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginParams {
    pub cli_port: Option<String>,
    pub provider: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackParams {
    pub cli_port: Option<String>,
}

/// GET /api/auth-login — initiate Neon Auth social sign-in.
///
/// Calls the Neon Auth `/sign-in/social` endpoint and redirects
/// the browser to the Google OAuth flow.
pub async fn login(State(_): State<AppState>, Query(params): Query<LoginParams>) -> Response {
    let neon_auth_url = match std::env::var("NEON_AUTH_URL") {
        Ok(url) => url,
        Err(_) => {
            return error_response(500, "NEON_AUTH_URL not configured");
        }
    };

    let provider = params.provider.as_deref().unwrap_or("google");
    let host = std::env::var("VERCEL_PROJECT_PRODUCTION_URL")
        .unwrap_or_else(|_| "temperkb.io".to_string());
    let callback_base = format!("https://{host}/api/auth-callback");
    let callback_url = match &params.cli_port {
        Some(port) => format!("{callback_base}?cli_port={port}"),
        None => callback_base,
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let res = match client
        .post(format!("{neon_auth_url}/sign-in/social"))
        .header("Content-Type", "application/json")
        .header("Origin", format!("https://{host}"))
        .json(&serde_json::json!({
            "provider": provider,
            "callbackURL": callback_url,
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return error_response(502, &format!("Neon Auth request failed: {e}"));
        }
    };

    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return error_response(502, &format!("Neon Auth error: {body}"));
    }

    let data: serde_json::Value = match res.json().await {
        Ok(d) => d,
        Err(e) => {
            return error_response(502, &format!("Neon Auth response parse error: {e}"));
        }
    };

    let redirect_url = match data.get("url").and_then(|v| v.as_str()) {
        Some(url) => url.to_string(),
        None => {
            return error_response(502, &format!("No redirect URL: {data}"));
        }
    };

    Redirect::temporary(&redirect_url).into_response()
}

/// GET /api/auth-callback — renders a page that fetches JWT from Neon Auth.
///
/// After Google sign-in, Neon Auth redirects here. Session cookies are
/// on the Neon Auth domain, so we render a client-side page that fetches
/// `/auth/token` with `credentials: include`.
pub async fn callback(
    State(_): State<AppState>,
    Query(params): Query<CallbackParams>,
) -> Html<String> {
    let neon_auth_url = std::env::var("NEON_AUTH_URL").unwrap_or_default();
    let cli_port = params.cli_port.as_deref().unwrap_or("");

    let cli_redirect = if cli_port.is_empty() {
        "null".to_string()
    } else {
        format!("\"http://localhost:{cli_port}/callback?token=\" + encodeURIComponent(jwt)")
    };

    let retry_url = if cli_port.is_empty() {
        "/api/auth-login".to_string()
    } else {
        format!("/api/auth-login?cli_port={cli_port}")
    };

    Html(format!(
        r#"<!DOCTYPE html>
<html><head><title>temper auth</title>
<style>
body {{ font-family: system-ui; max-width: 600px; margin: 40px auto; padding: 0 20px; color: #e0e0e0; background: #0f0f1a; }}
pre {{ background: #1a1a2e; color: #e0e0e0; padding: 16px; border-radius: 8px; overflow-x: auto; white-space: pre-wrap; word-break: break-all; }}
.success {{ color: #22c55e; }}
.error {{ color: #ef4444; }}
.loading {{ color: #a0a0b0; }}
button {{ background: #6366f1; color: white; border: none; padding: 8px 16px; border-radius: 6px; cursor: pointer; font-size: 14px; }}
button:hover {{ background: #4f46e5; }}
a {{ color: #6366f1; }}
</style></head>
<body>
<div id="loading">
  <h2 class="loading">Completing authentication...</h2>
  <p>Fetching token from Neon Auth...</p>
</div>
<div id="success" style="display:none">
  <h2 class="success">Authenticated!</h2>
  <p>Run this in your terminal:</p>
  <pre id="cmd"></pre>
  <button onclick="navigator.clipboard.writeText(document.getElementById('cmd').textContent)">Copy command</button>
  <p style="margin-top:24px;color:#888">You can close this tab after copying.</p>
</div>
<div id="error" style="display:none">
  <h2 class="error">Authentication Error</h2>
  <pre id="error-detail"></pre>
  <p><a href="{retry_url}">Try signing in again</a></p>
</div>
<script>
(async () => {{
  try {{
    const res = await fetch("{neon_auth_url}/token", {{
      credentials: "include",
      headers: {{ "Accept": "application/json" }}
    }});
    if (!res.ok) {{
      const body = await res.text();
      showError("Token request failed (" + res.status + ")\\n\\n" + (body || "No session found.") +
        "\\n\\nThis usually means the session cookies were not set. " +
        "Make sure third-party cookies are enabled for the Neon Auth domain.");
      return;
    }}
    const text = await res.text();
    let jwt = null;
    try {{
      const data = JSON.parse(text);
      jwt = data.token || data.access_token || data.jwt;
    }} catch {{
      if (text.startsWith("eyJ")) jwt = text.trim();
    }}
    if (!jwt) {{
      showError("No JWT found in response.\\n\\nResponse: " + text);
      return;
    }}
    const cliRedirect = {cli_redirect};
    if (cliRedirect) {{
      window.location.href = cliRedirect;
      return;
    }}
    document.getElementById("loading").style.display = "none";
    document.getElementById("success").style.display = "block";
    document.getElementById("cmd").textContent = "temper auth token " + jwt;
  }} catch (err) {{
    showError("Fetch error: " + err.message +
      "\\n\\nThis is likely a CORS issue. The Neon Auth service may need " +
      "to allow this origin.");
  }}
}})();
function showError(msg) {{
  document.getElementById("loading").style.display = "none";
  document.getElementById("error").style.display = "block";
  document.getElementById("error-detail").textContent = msg;
}}
</script>
</body></html>"#
    ))
}

fn error_response(status: u16, message: &str) -> Response {
    use axum::http::StatusCode;
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::json!({
        "error": { "code": "AUTH_ERROR", "message": message }
    });
    (
        status,
        [("content-type", "application/json")],
        body.to_string(),
    )
        .into_response()
}
