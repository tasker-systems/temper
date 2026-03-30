use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{ClientError, Result};

/// Persisted auth state written to `~/.config/temper/auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
}

/// Summary returned by `auth_status` — safe to display without exposing tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub provider: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub profile_id: Option<uuid::Uuid>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns `~/.config/temper/` (or the platform equivalent).
pub fn auth_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("temper")
}

/// Returns `~/.config/temper/auth.json`.
pub fn auth_json_path() -> PathBuf {
    auth_dir().join("auth.json")
}

// ---------------------------------------------------------------------------
// Load / save / clear — path-parameterised for testability
// ---------------------------------------------------------------------------

/// Load stored auth from an explicit path.
pub fn load_auth_from(path: &Path) -> Result<Option<StoredAuth>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let auth: StoredAuth = serde_json::from_slice(&bytes)?;
    Ok(Some(auth))
}

/// Save auth to an explicit path, creating parent dirs and setting mode 0o600 on Unix.
pub fn save_auth_to(auth: &StoredAuth, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(auth)?;
    fs::write(path, &json)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms)?;
    }

    Ok(())
}

/// Remove auth file at an explicit path (no-op if absent).
pub fn clear_auth_at(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Convenience wrappers using the default path
// ---------------------------------------------------------------------------

/// Load stored auth from the default location.
pub fn load_auth() -> Result<Option<StoredAuth>> {
    load_auth_from(&auth_json_path())
}

/// Save auth to the default location.
pub fn save_auth(auth: &StoredAuth) -> Result<()> {
    save_auth_to(auth, &auth_json_path())
}

/// Remove auth from the default location.
pub fn clear_auth() -> Result<()> {
    clear_auth_at(&auth_json_path())
}

/// Return a lightweight status struct (no token values exposed).
pub fn auth_status() -> Result<AuthStatus> {
    match load_auth()? {
        None => Ok(AuthStatus {
            authenticated: false,
            provider: None,
            expires_at: None,
            profile_id: None,
        }),
        Some(a) => Ok(AuthStatus {
            authenticated: true,
            provider: Some(a.provider),
            expires_at: Some(a.expires_at),
            profile_id: a.profile_id,
        }),
    }
}

// ---------------------------------------------------------------------------
// Token refresh
// ---------------------------------------------------------------------------

/// Returns `true` when the token expires within 5 minutes (or is already expired).
pub fn needs_refresh(auth: &StoredAuth) -> bool {
    Utc::now() + Duration::minutes(5) >= auth.expires_at
}

/// OAuth2 token response shape — only the fields we care about.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// POST a refresh-token grant and persist the updated [`StoredAuth`].
pub async fn refresh_token(
    auth: &StoredAuth,
    token_url: &str,
    client_id: &str,
) -> Result<StoredAuth> {
    let refresh = auth
        .refresh_token
        .as_deref()
        .ok_or(ClientError::TokenExpired)?;

    let client = reqwest::Client::new();
    let resp = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(ClientError::TokenExpired);
    }

    let tr: TokenResponse = resp.json().await?;

    let expires_at = Utc::now() + Duration::seconds(tr.expires_in.unwrap_or(3600) as i64);

    let updated = StoredAuth {
        provider: auth.provider.clone(),
        access_token: tr.access_token,
        refresh_token: tr.refresh_token.or_else(|| auth.refresh_token.clone()),
        expires_at,
        profile_id: auth.profile_id,
    };

    save_auth(&updated)?;
    Ok(updated)
}

/// Load auth, refresh if needed, and return a valid access token.
pub async fn get_valid_token(token_url: &str, client_id: &str) -> Result<String> {
    let auth = load_auth()?.ok_or(ClientError::NotAuthenticated)?;

    if needs_refresh(&auth) {
        let refreshed = refresh_token(&auth, token_url, client_id).await?;
        return Ok(refreshed.access_token);
    }

    Ok(auth.access_token)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_auth(expires_at: DateTime<Utc>) -> StoredAuth {
        StoredAuth {
            provider: "github".to_owned(),
            access_token: "tok_test".to_owned(),
            refresh_token: Some("rtok_test".to_owned()),
            expires_at,
            profile_id: None,
        }
    }

    // --- needs_refresh ---

    #[test]
    fn needs_refresh_false_when_expires_in_10_minutes() {
        let auth = make_auth(Utc::now() + Duration::minutes(10));
        assert!(!needs_refresh(&auth));
    }

    #[test]
    fn needs_refresh_true_when_expires_in_3_minutes() {
        let auth = make_auth(Utc::now() + Duration::minutes(3));
        assert!(needs_refresh(&auth));
    }

    #[test]
    fn needs_refresh_true_when_already_expired() {
        let auth = make_auth(Utc::now() - Duration::minutes(1));
        assert!(needs_refresh(&auth));
    }

    // --- save / load / clear roundtrip ---

    #[test]
    fn save_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        let original = make_auth(Utc::now() + Duration::hours(1));

        save_auth_to(&original, &path).unwrap();

        let loaded = load_auth_from(&path).unwrap().expect("should be Some");
        assert_eq!(loaded.provider, original.provider);
        assert_eq!(loaded.access_token, original.access_token);
        assert_eq!(loaded.refresh_token, original.refresh_token);
    }

    #[test]
    fn load_returns_none_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let result = load_auth_from(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn clear_auth_removes_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        let auth = make_auth(Utc::now() + Duration::hours(1));

        save_auth_to(&auth, &path).unwrap();
        assert!(path.exists());

        clear_auth_at(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn clear_auth_no_op_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("no_such_file.json");
        // Must not return an error.
        clear_auth_at(&path).unwrap();
    }

    // --- auth_status when no file ---
    // We test via the path-parameterised helpers since the default path may
    // already exist on a developer machine.

    #[test]
    fn status_unauthenticated_when_no_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        // No file written — load_auth_from should return None.
        let stored = load_auth_from(&path).unwrap();
        assert!(stored.is_none());

        // Simulate what auth_status() does with no stored auth.
        let status = AuthStatus {
            authenticated: stored.is_some(),
            provider: stored.as_ref().map(|a| a.provider.clone()),
            expires_at: stored.as_ref().map(|a| a.expires_at),
            profile_id: stored.as_ref().and_then(|a| a.profile_id),
        };
        assert!(!status.authenticated);
        assert!(status.provider.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_has_mode_0o600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        let auth = make_auth(Utc::now() + Duration::hours(1));

        save_auth_to(&auth, &path).unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "auth.json should be owner-read/write only");
    }
}
