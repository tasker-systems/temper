//! `temper auth` subcommands: login, logout, status.
//!
//! All output is JSON so the commands can be consumed programmatically.

use crate::error::Result;

/// Run the OAuth2 PKCE login flow, persist the token, and print auth status.
pub fn login() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
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
}

/// Clear stored credentials and print confirmation.
pub fn logout() -> Result<()> {
    temper_client::auth::clear_auth()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    println!("{{\"status\": \"logged_out\"}}");
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
