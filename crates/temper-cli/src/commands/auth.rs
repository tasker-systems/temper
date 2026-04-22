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
    let claims = temper_client::auth::parse_jwt_claims(jwt)
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
        access_token: jwt.to_string().into(),
        refresh_token: None,
        expires_at: claims.expires_at,
        profile_id: claims.profile_id,
        device_id: Some(device_id),
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
