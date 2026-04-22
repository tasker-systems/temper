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

// ---------------------------------------------------------------------------
// TokenStore — abstracts over "where does the auth state live"
// ---------------------------------------------------------------------------
//
// DiskTokenStore  — ~/.config/temper/auth.json (local CLI default).
// MemoryTokenStore — ephemeral, populated from env vars at session start
//                    (cloud mode / agent runners with no writable $HOME).
//
// Every function that previously called the free `load_auth` / `save_auth`
// functions must move to accepting `&dyn TokenStore` so the caller chooses
// the backing storage explicitly. `VaultState::Cloud` MUST use
// MemoryTokenStore; `DiskTokenStore` reaching a cloud session would write
// refreshed tokens to the agent's $HOME.

pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<StoredAuth>>;
    fn save(&self, auth: &StoredAuth) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

/// Disk-backed token store — the local CLI default. Wraps the existing
/// `load_auth_from` / `save_auth_to` / `clear_auth_at` helpers with a
/// configurable path (default: `auth_json_path()`).
#[derive(Debug, Clone)]
pub struct DiskTokenStore {
    path: std::path::PathBuf,
}

impl DiskTokenStore {
    /// Use `~/.config/temper/auth.json`.
    pub fn default_path() -> Self {
        Self {
            path: auth_json_path(),
        }
    }

    /// Use an explicit path (tests, non-default installs).
    pub fn at(path: std::path::PathBuf) -> Self {
        Self { path }
    }
}

impl TokenStore for DiskTokenStore {
    fn load(&self) -> Result<Option<StoredAuth>> {
        // Env-var path takes precedence even for DiskTokenStore — matches
        // current `load_auth()` semantics. Callers in cloud mode should use
        // MemoryTokenStore; this fallback exists for backward compatibility
        // with any tool that instantiates DiskTokenStore without checking
        // VaultState.
        if let Some(stored) = stored_auth_from_env()? {
            return Ok(Some(stored));
        }
        load_auth_from(&self.path)
    }

    fn save(&self, auth: &StoredAuth) -> Result<()> {
        save_auth_to(auth, &self.path)
    }

    fn clear(&self) -> Result<()> {
        clear_auth_at(&self.path)
    }
}

/// Ephemeral, in-memory token store. Constructed once at session start —
/// typically from `TEMPER_TOKEN` via [`MemoryTokenStore::from_env()`]. Saves
/// stay in memory only; [`clear`](TokenStore::clear) wipes the slot.
/// Deliberately does NOT derive `Serialize` — prevents future "log the whole
/// client state as JSON" accidents.
///
/// Cloning this store produces a second handle to the **same** underlying
/// slot — saves through one handle are visible to all clones.
#[derive(Clone)]
pub struct MemoryTokenStore {
    inner: std::sync::Arc<std::sync::RwLock<Option<StoredAuth>>>,
}

impl MemoryTokenStore {
    /// Empty store — for tests, or when the caller will `save()` immediately.
    pub fn empty() -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Pre-populated from [`StoredAuth`].
    pub fn with_auth(auth: StoredAuth) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::RwLock::new(Some(auth))),
        }
    }

    /// Initialize from env vars (`TEMPER_TOKEN` + `TEMPER_PROVIDER` +
    /// `TEMPER_DEVICE_ID`). Returns `Ok(None)` when `TEMPER_TOKEN` is unset
    /// — caller should fall back to disk or error per `VaultState`. Returns
    /// `Err(_)` when the env is set but malformed.
    ///
    /// Unlike [`stored_auth_from_env`], this reads the env **once**; later
    /// [`load`](TokenStore::load) calls return the in-memory state, not a
    /// fresh env read.
    pub fn from_env() -> Result<Option<Self>> {
        match stored_auth_from_env()? {
            Some(auth) => Ok(Some(Self::with_auth(auth))),
            None => Ok(None),
        }
    }

    /// Like [`from_env`](Self::from_env), but errors when `TEMPER_TOKEN` is
    /// unset — the expected shape for cloud-mode dispatch where no disk
    /// fallback is valid. Centralizes the canonical error message so it
    /// can't drift across call sites.
    pub fn from_env_required() -> Result<Self> {
        Self::from_env()?.ok_or_else(|| {
            ClientError::Other("TEMPER_VAULT_STATE=cloud but TEMPER_TOKEN is not set".into())
        })
    }
}

impl std::fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state: &dyn std::fmt::Debug = match self.inner.read() {
            Ok(g) => &g.is_some(),
            Err(_) => &"<poisoned>",
        };
        f.debug_struct("MemoryTokenStore")
            .field("populated", state)
            .finish()
    }
}

impl TokenStore for MemoryTokenStore {
    // Poison recovery: a panic in an unrelated thread (OOM, agent-runner
    // SIGSEGV, etc.) will poison the RwLock, but the token itself is still
    // valid. The invariant here is "single `Option<StoredAuth>` cell" — a
    // poisoned lock just means a previous guard was dropped during a panic.
    // `PoisonError::into_inner()` returns the guard regardless, so we recover
    // rather than killing every subsequent call. No dedicated unit test for
    // this path: simulating a panic-poisoned RwLock in a test is fiddly and
    // low-value relative to the existing happy-path coverage.
    fn load(&self) -> Result<Option<StoredAuth>> {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        Ok(guard.clone())
    }

    fn save(&self, auth: &StoredAuth) -> Result<()> {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = Some(auth.clone());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = None;
        Ok(())
    }
}

/// Default Auth0 tenant domain used when no explicit domain is supplied via
/// `TEMPER_PROVIDER`. Matches `CloudSection::default()` in `temper-core`.
/// Kept as a compile-time constant to avoid an I/O dependency in the env-var
/// bootstrap path — see also `default_provider_is_auth0_with_config` in
/// `config::tests` which pins the same value.
const DEFAULT_AUTH0_DOMAIN: &str = "temperkb.us.auth0.com";

/// Expose the default Auth0 domain for consumers that need to construct a
/// `Provider::Auth0` without re-reading config.
pub fn default_auth0_domain() -> String {
    DEFAULT_AUTH0_DOMAIN.to_string()
}

/// Which identity provider issued the stored credentials.
///
/// One variant today; the enum shape is the change — a second variant
/// (hypothetical `SelfHosted`, etc.) is a separate design question and is
/// NOT speculatively built. Extending the enum later is a minor refactor;
/// extending a stringly-typed field is a breaking API change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provider {
    Auth0 { domain: String },
}

impl Provider {
    /// Convenience constructor for the common case.
    pub fn auth0(domain: impl Into<String>) -> Self {
        Self::Auth0 {
            domain: domain.into(),
        }
    }

    /// Expose the provider's domain (e.g. `temper.us.auth0.com`) for OAuth
    /// endpoint construction. The domain is informational on the token side
    /// — actual OAuth endpoints come from `config.toml`'s `[[auth.providers]]`.
    pub fn domain(&self) -> &str {
        match self {
            Provider::Auth0 { domain } => domain,
        }
    }

    /// Short identifier used in status displays (`temper auth status`).
    pub fn kind(&self) -> &'static str {
        match self {
            Provider::Auth0 { .. } => "auth0",
        }
    }

    /// Parse the `TEMPER_PROVIDER` env-var value, treating unknown/invalid
    /// values as a fallback to the default Auth0 tenant. Prefer
    /// [`try_from_env_value`](Self::try_from_env_value) when unknown values
    /// should surface as an error instead.
    ///
    /// Accepted shapes:
    /// - `None` / empty / `"auth0"` → `Auth0 { default_auth0_domain() }`
    /// - `"auth0:DOMAIN"` → `Auth0 { DOMAIN }`
    /// - anything else → falls back to the default Auth0 tenant
    pub fn from_env_value(raw: Option<&str>) -> Self {
        Self::try_from_env_value(raw).unwrap_or_else(|_| Self::auth0(default_auth0_domain()))
    }

    /// Strict parse of `TEMPER_PROVIDER`: unknown shapes return an error so
    /// the CLI can surface a clear message rather than silently defaulting.
    pub fn try_from_env_value(raw: Option<&str>) -> Result<Self> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("auth0") => Ok(Self::auth0(default_auth0_domain())),
            Some(s) if s.starts_with("auth0:") => {
                Ok(Self::auth0(s.trim_start_matches("auth0:").to_string()))
            }
            Some(other) => Err(ClientError::Other(format!(
                "unsupported TEMPER_PROVIDER value: {other}"
            ))),
        }
    }
}

/// Persisted auth state written to `~/.config/temper/auth.json`.
///
/// `device_id` is a UUIDv7 generated on first login, identifying this machine.
/// It is sent as `X-Temper-Device-Id` on every API request and used for
/// per-device sync state and vault config overrides.
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub provider: Provider,
    #[serde(with = "secrecy_serde")]
    pub access_token: secrecy::SecretString,
    #[serde(default, with = "secrecy_serde_opt")]
    pub refresh_token: Option<secrecy::SecretString>,
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
    /// Per-device identity — generated once on first login, stable across re-auth.
    #[serde(default)]
    pub device_id: Option<String>,
}

impl std::fmt::Debug for StoredAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredAuth")
            .field("provider", &self.provider)
            .field("access_token", &"<REDACTED>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<REDACTED>"),
            )
            .field("expires_at", &self.expires_at)
            .field("profile_id", &self.profile_id)
            .field("device_id", &self.device_id)
            .finish()
    }
}

/// Serde helper: serialize/deserialize `SecretString` as its underlying
/// string. The JSON layer is trusted because `auth.json` is chmod 0o600
/// and the env-var path never reaches serde at all.
mod secrecy_serde {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(s: &SecretString, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(s.expose_secret())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
        String::deserialize(d).map(SecretString::from)
    }
}

mod secrecy_serde_opt {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(s: &Option<SecretString>, ser: S) -> Result<S::Ok, S::Error> {
        match s {
            Some(v) => ser.serialize_some(v.expose_secret()),
            None => ser.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SecretString>, D::Error> {
        Option::<String>::deserialize(d).map(|o| o.map(SecretString::from))
    }
}

/// Summary returned by `auth_status` — safe to display without exposing tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub provider: Option<Provider>,
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
/// Read-only; safe in cloud mode. Writers must go through [`TokenStore`] so
/// `MemoryTokenStore` sessions cannot accidentally persist to disk — see
/// module-level comment.
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

/// Return a lightweight status struct (no token values exposed).
///
/// Accepts a [`TokenStore`] so cloud sessions (backed by
/// [`MemoryTokenStore`]) report the in-memory auth and disk sessions report
/// `~/.config/temper/auth.json`. There is deliberately no `auth_status()`
/// free function that hardcodes the disk path.
pub fn auth_status(store: &dyn TokenStore) -> Result<AuthStatus> {
    match store.load()? {
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
    /// Only populated when `sub` parses as a UUID (our issuer's profile id).
    /// Unit D tokens issued via Auth0 Management API may have non-UUID
    /// subjects (e.g., `auth0|abc123`); use `sub` for the raw value.
    pub profile_id: Option<uuid::Uuid>,
    /// The raw `sub` claim as a string, regardless of format. Preserved
    /// for downstream callers (tracing, Unit D session-id extraction) that
    /// need to distinguish "sub missing" from "sub present but not a UUID".
    pub sub: Option<String>,
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

    let sub = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let profile_id = sub.as_deref().and_then(|s| uuid::Uuid::parse_str(s).ok());

    Ok(JwtClaims {
        expires_at,
        profile_id,
        sub,
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

    let provider_env = std::env::var(TEMPER_PROVIDER_ENV).ok();
    let provider = Provider::try_from_env_value(provider_env.as_deref())?;

    let device_id = std::env::var(TEMPER_DEVICE_ID_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    Ok(Some(StoredAuth {
        provider,
        access_token: jwt.into(),
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

/// POST a refresh-token grant and persist the updated [`StoredAuth`] via
/// the provided [`TokenStore`].
///
/// Takes a store rather than writing to the disk path directly — a cloud
/// session using [`MemoryTokenStore`] refreshes into memory, a local
/// session refreshes to `~/.config/temper/auth.json`. There is no
/// `refresh_token` free function that hardcodes disk — structural, not
/// discipline-based.
pub async fn refresh_token(
    store: &dyn TokenStore,
    auth: &StoredAuth,
    token_url: &str,
    client_id: &str,
) -> Result<StoredAuth> {
    use secrecy::ExposeSecret;

    let refresh = auth
        .refresh_token
        .as_ref()
        .ok_or(ClientError::TokenExpired)?;

    let client = reqwest::Client::new();
    let resp = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.expose_secret()),
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
        access_token: tr.access_token.into(),
        refresh_token: tr
            .refresh_token
            .map(Into::into)
            .or_else(|| auth.refresh_token.clone()),
        expires_at,
        profile_id: auth.profile_id,
        device_id: auth.device_id.clone(),
    };

    store.save(&updated)?;
    Ok(updated)
}

/// Load auth from the store, refresh if needed, and return a valid access
/// token string.
///
/// The store is the single source of truth for where tokens live — cloud
/// sessions use [`MemoryTokenStore`], local sessions use [`DiskTokenStore`].
/// Callers never reach for the disk path directly.
pub async fn get_valid_token(
    store: &dyn TokenStore,
    token_url: &str,
    client_id: &str,
) -> Result<String> {
    use secrecy::ExposeSecret;

    let auth = store.load()?.ok_or(ClientError::NotAuthenticated)?;

    if needs_refresh(&auth) {
        let refreshed = refresh_token(store, &auth, token_url, client_id).await?;
        return Ok(refreshed.access_token.expose_secret().to_string());
    }

    if auth.expires_at <= Utc::now() {
        return Err(ClientError::TokenExpired);
    }

    Ok(auth.access_token.expose_secret().to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use tempfile::TempDir;

    fn make_auth(expires_at: DateTime<Utc>) -> StoredAuth {
        StoredAuth {
            provider: Provider::auth0("test.auth0.com"),
            access_token: "tok_test".to_owned().into(),
            refresh_token: Some("rtok_test".to_owned().into()),
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
        assert_eq!(
            loaded.access_token.expose_secret(),
            original.access_token.expose_secret()
        );
        assert_eq!(
            loaded.refresh_token.as_ref().map(|s| s.expose_secret()),
            original.refresh_token.as_ref().map(|s| s.expose_secret())
        );
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

    /// Plan-named alias over `fake_jwt`: kept distinct so future changes to
    /// the "payload shaped test JWT" signature don't ripple into fake_jwt's
    /// call sites. Both helpers delegate to the same base64url encoding.
    fn make_test_jwt_with_payload(payload: &serde_json::Value) -> String {
        fake_jwt(payload)
    }

    #[test]
    fn parse_jwt_claims_preserves_non_uuid_sub() {
        // Issuer pattern "auth0|abc123" — not a UUID.
        let payload = serde_json::json!({
            "exp": (Utc::now() + Duration::hours(1)).timestamp(),
            "sub": "auth0|6123abcdef"
        });
        let jwt = make_test_jwt_with_payload(&payload);

        let claims = parse_jwt_claims(&jwt).expect("parse");
        assert!(
            claims.profile_id.is_none(),
            "non-UUID sub: profile_id stays None"
        );
        assert_eq!(
            claims.sub.as_deref(),
            Some("auth0|6123abcdef"),
            "raw sub must be preserved for downstream (Unit D) callers"
        );
    }

    #[test]
    fn parse_jwt_claims_uuid_sub_populates_both_fields() {
        let uuid = uuid::Uuid::now_v7();
        let payload = serde_json::json!({
            "exp": (Utc::now() + Duration::hours(1)).timestamp(),
            "sub": uuid.to_string()
        });
        let jwt = make_test_jwt_with_payload(&payload);

        let claims = parse_jwt_claims(&jwt).expect("parse");
        assert_eq!(claims.profile_id, Some(uuid));
        assert_eq!(claims.sub.as_deref(), Some(uuid.to_string().as_str()));
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
        std::env::set_var(TEMPER_PROVIDER_ENV, "auth0:test.auth0.com");
        std::env::set_var(TEMPER_DEVICE_ID_ENV, "fixed-device");

        let stored = stored_auth_from_env()
            .unwrap()
            .expect("env should produce auth");
        clear_env();

        assert_eq!(stored.access_token.expose_secret(), jwt);
        assert_eq!(stored.provider, Provider::auth0("test.auth0.com"));
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

        assert_eq!(stored.provider, Provider::auth0(default_auth0_domain()));
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
    fn disk_token_store_round_trips_stored_auth() {
        let tmp = tempfile::NamedTempFile::new().expect("tmpfile");
        let store = DiskTokenStore::at(tmp.path().to_path_buf());

        let auth = StoredAuth {
            provider: Provider::auth0(default_auth0_domain()),
            access_token: "at_test".to_string().into(),
            refresh_token: Some("rt_test".to_string().into()),
            expires_at: Utc::now() + Duration::hours(1),
            profile_id: Some(uuid::Uuid::now_v7()),
            device_id: Some(uuid::Uuid::now_v7().to_string()),
        };

        store.save(&auth).expect("save");
        let loaded = store.load().expect("load").expect("some");
        assert_eq!(
            loaded.access_token.expose_secret(),
            auth.access_token.expose_secret()
        );
        assert_eq!(
            loaded.refresh_token.as_ref().map(|s| s.expose_secret()),
            auth.refresh_token.as_ref().map(|s| s.expose_secret())
        );
        assert_eq!(loaded.provider, auth.provider);

        store.clear().expect("clear");
        let after_clear = store.load().expect("load after clear");
        // Env var may still populate on CI — guard against false failure.
        if std::env::var("TEMPER_TOKEN").is_err() {
            assert!(after_clear.is_none());
        }
    }

    #[test]
    fn stored_auth_debug_redacts_tokens() {
        let auth = StoredAuth {
            provider: Provider::auth0(default_auth0_domain()),
            access_token: secrecy::SecretString::from("at_sensitive"),
            refresh_token: Some(secrecy::SecretString::from("rt_sensitive")),
            expires_at: Utc::now(),
            profile_id: None,
            device_id: None,
        };
        let rendered = format!("{auth:?}");
        assert!(
            !rendered.contains("at_sensitive"),
            "access_token must not appear in Debug output: {rendered}"
        );
        assert!(
            !rendered.contains("rt_sensitive"),
            "refresh_token must not appear in Debug output: {rendered}"
        );
        assert!(
            rendered.contains("provider"),
            "structural fields still present: {rendered}"
        );
    }

    #[test]
    fn memory_token_store_returns_what_was_saved_not_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();

        let store = MemoryTokenStore::empty();
        assert!(store.load().expect("load").is_none());

        let auth = StoredAuth {
            provider: Provider::auth0(default_auth0_domain()),
            access_token: secrecy::SecretString::from("at_v1"),
            refresh_token: None,
            expires_at: Utc::now() + Duration::hours(1),
            profile_id: None,
            device_id: None,
        };
        store.save(&auth).expect("save");

        let loaded = store.load().expect("load").expect("some");
        assert_eq!(loaded.access_token.expose_secret(), "at_v1");

        store.clear().expect("clear");
        assert!(store.load().expect("load after clear").is_none());
    }

    #[test]
    fn memory_token_store_from_env_reads_token_once() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();

        let exp = (Utc::now() + Duration::hours(1)).timestamp();
        let sub = uuid::Uuid::now_v7();
        let jwt = fake_jwt(&serde_json::json!({
            "exp": exp,
            "sub": sub.to_string(),
        }));
        std::env::set_var(TEMPER_TOKEN_ENV, &jwt);

        let store = MemoryTokenStore::from_env()
            .expect("from_env")
            .expect("some");

        // Mutate env after construction — store must NOT re-read.
        std::env::set_var(TEMPER_TOKEN_ENV, "junk_not_a_jwt");

        let loaded = store.load().expect("load").expect("some");
        // Token came from the initial parse, not from the junk env.
        assert_eq!(loaded.provider, Provider::auth0(default_auth0_domain()));
        assert_eq!(loaded.access_token.expose_secret(), jwt);

        clear_env();
    }

    #[test]
    fn memory_token_store_from_env_required_errors_without_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let err = MemoryTokenStore::from_env_required().unwrap_err();
        assert!(
            matches!(err, ClientError::Other(_)),
            "expected ClientError::Other, got {err:?}"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("TEMPER_TOKEN"),
            "error mentions TEMPER_TOKEN: {msg}"
        );
    }

    #[test]
    fn provider_auth0_serializes_as_tagged_enum() {
        let p = Provider::Auth0 {
            domain: "temperkb.us.auth0.com".to_string(),
        };
        let json = serde_json::to_string(&p).expect("to_string");
        assert!(json.contains("\"kind\":\"auth0\""), "tag present: {json}");
        assert!(
            json.contains("\"domain\":\"temperkb.us.auth0.com\""),
            "domain present: {json}"
        );

        let round: Provider = serde_json::from_str(&json).expect("from_str");
        assert_eq!(round, p);
    }

    #[test]
    fn provider_parses_from_env_shapes() {
        assert_eq!(
            Provider::from_env_value(None),
            Provider::auth0(default_auth0_domain())
        );
        assert_eq!(
            Provider::from_env_value(Some("")),
            Provider::auth0(default_auth0_domain())
        );
        assert_eq!(
            Provider::from_env_value(Some("auth0")),
            Provider::auth0(default_auth0_domain())
        );
        assert_eq!(
            Provider::from_env_value(Some("auth0:my.domain.com")),
            Provider::auth0("my.domain.com")
        );
        assert!(
            Provider::from_env_value(Some("github")) == Provider::auth0(default_auth0_domain())
                || Provider::try_from_env_value(Some("github")).is_err()
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
        assert_eq!(loaded.access_token.expose_secret(), jwt);
        assert_eq!(loaded.profile_id, Some(env_profile));
        assert!(loaded.refresh_token.is_none());
    }
}
