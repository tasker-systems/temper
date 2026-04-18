use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{ClientError, Result};

/// Environment variable names honored by cloud-mode / ephemeral sessions.
///
/// See `docs/superpowers/specs/2026-04-18-cloud-mode-and-portable-memory-design.md`
/// for the design rationale.
pub const TEMPER_TOKEN_ENV: &str = "TEMPER_TOKEN";
pub const TEMPER_PROVIDER_ENV: &str = "TEMPER_PROVIDER";
pub const TEMPER_DEVICE_ID_ENV: &str = "TEMPER_DEVICE_ID";

/// Persisted auth state written to `~/.config/temper/auth.json`.
///
/// `device_id` is a UUIDv7 generated on first login, identifying this machine.
/// It is sent as `X-Temper-Device-Id` on every API request and used for
/// per-device sync state and vault config overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
    /// Per-device identity — generated once on first login, stable across re-auth.
    #[serde(default)]
    pub device_id: Option<String>,
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

/// Returns `~/.config/temper/`.
pub fn auth_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config")
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

/// Load stored auth, preferring `TEMPER_TOKEN` when set, falling back to
/// `~/.config/temper/auth.json`.
///
/// When `TEMPER_TOKEN` is set, the token is parsed into an in-memory
/// [`StoredAuth`] without touching disk — the primary bootstrap path for
/// ephemeral cloud agent sessions. A malformed `TEMPER_TOKEN` surfaces as an
/// error rather than silently falling through to disk auth; an explicit env
/// var is an explicit instruction.
pub fn load_auth() -> Result<Option<StoredAuth>> {
    if let Some(stored) = stored_auth_from_env()? {
        return Ok(Some(stored));
    }
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
// JWT parsing + env-var bootstrap
// ---------------------------------------------------------------------------

/// Claims extracted from a JWT payload — only the fields we care about for
/// building a [`StoredAuth`].
#[derive(Debug, Clone)]
pub struct JwtClaims {
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
}

/// Decode a JWT payload (base64url → JSON) and extract `exp` and `sub` claims.
///
/// The signature is not verified — the client does not hold the signing key.
/// Signature verification happens at the API, which is the authoritative trust
/// boundary. The client's interest in the JWT is strictly mechanical: learn
/// when the token expires and which profile it belongs to.
pub fn parse_jwt_claims(jwt: &str) -> Result<JwtClaims> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(ClientError::Other(
            "invalid JWT format — expected header.payload.signature".into(),
        ));
    }

    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload_bytes = engine
        .decode(parts[1])
        .map_err(|e| ClientError::Other(format!("JWT decode error: {e}")))?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|e| ClientError::Other(format!("JWT payload parse error: {e}")))?;

    let exp = payload
        .get("exp")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ClientError::Other("JWT missing 'exp' claim".into()))?;

    let expires_at = DateTime::from_timestamp(exp, 0)
        .ok_or_else(|| ClientError::Other("invalid 'exp' timestamp in JWT".into()))?;

    let profile_id = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    Ok(JwtClaims {
        expires_at,
        profile_id,
    })
}

/// If `TEMPER_TOKEN` is set, build an in-memory [`StoredAuth`] from it.
///
/// Returns `Ok(None)` when `TEMPER_TOKEN` is unset or empty — the caller then
/// falls back to disk-backed auth. Returns `Err(_)` when the env var is set
/// but malformed.
///
/// Provider defaults to `"auth0"` when `TEMPER_PROVIDER` is unset (matches
/// the out-of-box config default). Device id is taken from `TEMPER_DEVICE_ID`
/// when set; otherwise a fresh UUIDv7 is generated for this session — per
/// the cloud-mode design, the session is ephemeral and a fresh device id is
/// acceptable.
///
/// The returned `StoredAuth` has `refresh_token: None` — env-var auth is
/// intentionally refresh-less in this pass. When the token approaches expiry
/// the caller receives `ClientError::TokenExpired` and must re-export a fresh
/// token. Refresh semantics for cloud sessions are Unit B.4 research work,
/// not B.1.
pub fn stored_auth_from_env() -> Result<Option<StoredAuth>> {
    let jwt = match std::env::var(TEMPER_TOKEN_ENV) {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(None),
    };

    let claims = parse_jwt_claims(&jwt)?;

    let provider = std::env::var(TEMPER_PROVIDER_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "auth0".to_string());

    let device_id = std::env::var(TEMPER_DEVICE_ID_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    Ok(Some(StoredAuth {
        provider,
        access_token: jwt,
        refresh_token: None,
        expires_at: claims.expires_at,
        profile_id: claims.profile_id,
        device_id: Some(device_id),
    }))
}

// ---------------------------------------------------------------------------
// Device ID helper
// ---------------------------------------------------------------------------

/// Load an existing device_id from auth.json, or generate a new UUIDv7.
///
/// Called during login to ensure every auth.json has a stable device_id.
/// If the user already has one (re-login), it is preserved.
pub fn load_or_create_device_id() -> String {
    if let Ok(Some(auth)) = load_auth() {
        if let Some(id) = auth.device_id {
            if !id.is_empty() {
                return id;
            }
        }
    }
    uuid::Uuid::now_v7().to_string()
}

/// Load the device_id from auth.json.
///
/// Returns `None` if not authenticated or if the stored auth predates
/// the device_id field.
pub fn load_device_id() -> Option<String> {
    let auth = load_auth().ok()??;
    auth.device_id.filter(|id| !id.is_empty())
}

// ---------------------------------------------------------------------------
// Current token helper
// ---------------------------------------------------------------------------

/// Load the stored access token, returning an error if not authenticated
/// or if the token has expired.
///
/// This is the primary helper used by sub-clients to get a bearer token
/// for outgoing requests without needing access to the OAuth config.
pub fn current_token() -> Result<String> {
    let auth = load_auth()?.ok_or(ClientError::NotAuthenticated)?;
    if auth.expires_at <= Utc::now() {
        return Err(ClientError::TokenExpired);
    }
    Ok(auth.access_token)
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
        device_id: auth.device_id.clone(),
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
            device_id: Some("test-device-id".to_owned()),
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

    // --- JWT parsing + env-var bootstrap ---

    use std::sync::Mutex;

    /// Serialize tests that mutate TEMPER_TOKEN / TEMPER_PROVIDER / TEMPER_DEVICE_ID.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Build a fake JWT with the given JSON claims object as payload. The
    /// signature is ignored by the parser so we use a placeholder string.
    fn fake_jwt(claims: &serde_json::Value) -> String {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = engine.encode(claims.to_string().as_bytes());
        format!("{header}.{payload}.sig")
    }

    fn clear_env() {
        std::env::remove_var(TEMPER_TOKEN_ENV);
        std::env::remove_var(TEMPER_PROVIDER_ENV);
        std::env::remove_var(TEMPER_DEVICE_ID_ENV);
    }

    #[test]
    fn parse_jwt_extracts_exp_and_sub() {
        let exp = (Utc::now() + Duration::hours(1)).timestamp();
        let sub = uuid::Uuid::now_v7();
        let jwt = fake_jwt(&serde_json::json!({
            "exp": exp,
            "sub": sub.to_string(),
        }));

        let claims = parse_jwt_claims(&jwt).unwrap();
        assert_eq!(claims.expires_at.timestamp(), exp);
        assert_eq!(claims.profile_id, Some(sub));
    }

    #[test]
    fn parse_jwt_profile_id_none_when_sub_not_uuid() {
        let exp = (Utc::now() + Duration::hours(1)).timestamp();
        let jwt = fake_jwt(&serde_json::json!({
            "exp": exp,
            "sub": "not-a-uuid",
        }));

        let claims = parse_jwt_claims(&jwt).unwrap();
        assert!(claims.profile_id.is_none());
    }

    #[test]
    fn parse_jwt_rejects_wrong_part_count() {
        let err = parse_jwt_claims("only.two").unwrap_err();
        assert!(
            matches!(err, ClientError::Other(ref s) if s.contains("expected header.payload.signature"))
        );
    }

    #[test]
    fn parse_jwt_rejects_missing_exp() {
        let jwt = fake_jwt(&serde_json::json!({
            "sub": "anything",
        }));
        let err = parse_jwt_claims(&jwt).unwrap_err();
        assert!(matches!(err, ClientError::Other(ref s) if s.contains("'exp'")));
    }

    #[test]
    fn env_bootstrap_none_when_token_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let result = stored_auth_from_env().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn env_bootstrap_none_when_token_empty() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        std::env::set_var(TEMPER_TOKEN_ENV, "");
        let result = stored_auth_from_env().unwrap();
        clear_env();
        assert!(result.is_none());
    }

    #[test]
    fn env_bootstrap_builds_in_memory_auth() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let exp = (Utc::now() + Duration::hours(2)).timestamp();
        let sub = uuid::Uuid::now_v7();
        let jwt = fake_jwt(&serde_json::json!({
            "exp": exp,
            "sub": sub.to_string(),
        }));
        std::env::set_var(TEMPER_TOKEN_ENV, &jwt);
        std::env::set_var(TEMPER_PROVIDER_ENV, "auth0-test");
        std::env::set_var(TEMPER_DEVICE_ID_ENV, "fixed-device");

        let stored = stored_auth_from_env()
            .unwrap()
            .expect("env should produce auth");
        clear_env();

        assert_eq!(stored.access_token, jwt);
        assert_eq!(stored.provider, "auth0-test");
        assert_eq!(stored.device_id.as_deref(), Some("fixed-device"));
        assert_eq!(stored.profile_id, Some(sub));
        assert!(
            stored.refresh_token.is_none(),
            "env-var auth is refresh-less"
        );
    }

    #[test]
    fn env_bootstrap_defaults_provider_and_generates_device_id() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let exp = (Utc::now() + Duration::hours(1)).timestamp();
        let jwt = fake_jwt(&serde_json::json!({
            "exp": exp,
            "sub": uuid::Uuid::now_v7().to_string(),
        }));
        std::env::set_var(TEMPER_TOKEN_ENV, &jwt);

        let stored = stored_auth_from_env()
            .unwrap()
            .expect("env should produce auth");
        clear_env();

        assert_eq!(stored.provider, "auth0");
        let device_id = stored.device_id.expect("device_id should be generated");
        uuid::Uuid::parse_str(&device_id).expect("generated device_id should parse as UUID");
    }

    #[test]
    fn env_bootstrap_surfaces_malformed_token_as_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        std::env::set_var(TEMPER_TOKEN_ENV, "this.is.not-valid");
        let result = stored_auth_from_env();
        clear_env();
        assert!(
            result.is_err(),
            "malformed TEMPER_TOKEN must error, not fall through"
        );
    }

    #[test]
    fn load_auth_prefers_env_over_disk() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();

        // Build env-var JWT with a recognizable profile_id.
        let env_profile = uuid::Uuid::now_v7();
        let exp = (Utc::now() + Duration::hours(1)).timestamp();
        let jwt = fake_jwt(&serde_json::json!({
            "exp": exp,
            "sub": env_profile.to_string(),
        }));
        std::env::set_var(TEMPER_TOKEN_ENV, &jwt);

        let loaded = load_auth().unwrap().expect("should load env-var auth");
        clear_env();

        // Env-var path is distinguishable by refresh_token=None and matching JWT.
        assert_eq!(loaded.access_token, jwt);
        assert_eq!(loaded.profile_id, Some(env_profile));
        assert!(loaded.refresh_token.is_none());
    }
}
