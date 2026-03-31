//! `temper auth` subcommands: login, logout, status.
//!
//! All output is JSON so the commands can be consumed programmatically.

use crate::actions::runtime;
use crate::error::Result;

/// Run the OAuth2 PKCE login flow, persist the token, and print auth status.
pub fn login() -> Result<()> {
    runtime::with_client(|client| {
        Box::pin(async move {
            let stored = client
                .auth_login()
                .await
                .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
            let status = temper_client::auth::AuthStatus {
                authenticated: true,
                provider: Some(stored.provider),
                expires_at: Some(stored.expires_at),
                profile_id: stored.profile_id,
            };
            let json =
                serde_json::to_string_pretty(&status).map_err(crate::error::TemperError::Json)?;
            println!("{json}");
            Ok(())
        })
    })
}

/// Clear stored credentials and print confirmation.
pub fn logout() -> Result<()> {
    temper_client::auth::clear_auth()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    println!("{{\"status\": \"logged_out\"}}");
    Ok(())
}

/// Store a JWT directly, bypassing the OAuth flow.
///
/// Useful for API-only clients, CI environments, or bootstrapping
/// when the browser OAuth flow isn't available yet.
pub fn token(jwt: &str, provider: &str) -> Result<()> {
    // Decode the JWT payload to extract expiry (without verifying signature)
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(crate::error::TemperError::Config(
            "invalid JWT format — expected header.payload.signature".into(),
        ));
    }

    // Decode the payload (base64url → JSON)
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload_bytes = engine
        .decode(parts[1])
        .map_err(|e| crate::error::TemperError::Config(format!("JWT decode error: {e}")))?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|e| crate::error::TemperError::Config(format!("JWT payload parse error: {e}")))?;

    let exp = payload
        .get("exp")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| crate::error::TemperError::Config("JWT missing 'exp' claim".into()))?;

    let expires_at = chrono::DateTime::from_timestamp(exp, 0).ok_or_else(|| {
        crate::error::TemperError::Config("invalid 'exp' timestamp in JWT".into())
    })?;

    let profile_id = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    let stored = temper_client::auth::StoredAuth {
        provider: provider.to_string(),
        access_token: jwt.to_string(),
        refresh_token: None,
        expires_at,
        profile_id,
    };

    temper_client::auth::save_auth(&stored)
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

    let status = temper_client::auth::AuthStatus {
        authenticated: true,
        provider: Some(stored.provider),
        expires_at: Some(stored.expires_at),
        profile_id: stored.profile_id,
    };
    let json = serde_json::to_string_pretty(&status).map_err(crate::error::TemperError::Json)?;
    println!("{json}");
    Ok(())
}

/// Print the current auth status as JSON.
pub fn status() -> Result<()> {
    let status = temper_client::auth::auth_status()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    let json = serde_json::to_string_pretty(&status).map_err(crate::error::TemperError::Json)?;
    println!("{json}");
    Ok(())
}
