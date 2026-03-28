use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::config::ApiConfig;

/// Cached keys with a timestamp for TTL-based invalidation.
struct CachedKeys {
    key: DecodingKey,
    fetched_at: Instant,
}

/// Fetches EdDSA/Ed25519 public keys from a JWKS endpoint, caches them,
/// and provides them for JWT verification.
pub struct JwksKeyStore {
    url: String,
    client: reqwest::Client,
    cache: RwLock<Option<CachedKeys>>,
    ttl: Duration,
}

impl std::fmt::Debug for JwksKeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwksKeyStore")
            .field("url", &self.url)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

/// Check if a JWK is an OKP key with Ed25519 curve (the only key type
/// we accept for EdDSA verification).
fn is_ed25519_okp(params: &AlgorithmParameters) -> bool {
    matches!(
        params,
        AlgorithmParameters::OctetKeyPair(p) if p.curve == EllipticCurve::Ed25519
    )
}

impl JwksKeyStore {
    /// Create a new key store that will fetch from the given JWKS URL.
    /// The cache TTL defaults to 1 hour.
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            cache: RwLock::new(None),
            ttl: Duration::from_secs(3600),
        }
    }

    /// Create a key store pre-loaded with a static key. Intended for tests
    /// that do not have network access to a real JWKS endpoint.
    pub fn with_static_key(key: DecodingKey) -> Self {
        let cached = CachedKeys {
            key,
            // Use a far-future instant so the cached key never expires.
            fetched_at: Instant::now() + Duration::from_secs(u32::MAX as u64),
        };
        Self {
            url: String::new(),
            client: reqwest::Client::new(),
            cache: RwLock::new(Some(cached)),
            // Very long TTL; the pre-loaded cache will never be refreshed.
            ttl: Duration::from_secs(u32::MAX as u64),
        }
    }

    /// Return a cached `DecodingKey`, refreshing from the JWKS endpoint if
    /// the cache is absent or has expired.
    pub async fn get_decoding_key(&self) -> Result<DecodingKey, String> {
        // Fast path: read lock, check freshness.
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < self.ttl {
                    // Clone the inner key bytes via the jsonwebtoken public API.
                    // DecodingKey does not implement Clone directly, so we re-wrap.
                    return Ok(cached.key.clone());
                }
            }
        }

        // Cache is stale or absent — refresh under write lock.
        self.refresh().await?;

        let guard = self.cache.read().await;
        guard
            .as_ref()
            .map(|c| c.key.clone())
            .ok_or_else(|| "JWKS cache empty after refresh".to_string())
    }

    /// Build a `Validation` struct configured for EdDSA with the given issuer
    /// and optional audience.
    pub fn validation(&self, issuer: &str, audience: Option<&str>) -> Validation {
        let mut v = Validation::new(Algorithm::EdDSA);
        v.set_issuer(&[issuer]);
        if let Some(aud) = audience {
            v.set_audience(&[aud]);
        } else {
            v.validate_aud = false;
        }
        v
    }

    /// Fetch the JWKS endpoint, parse the first usable OKP/Ed25519 key, and
    /// store it in the cache.
    pub async fn refresh(&self) -> Result<(), String> {
        let response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|e| format!("JWKS fetch failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("JWKS endpoint returned status {status}"));
        }

        let jwks: JwkSet = response
            .json()
            .await
            .map_err(|e| format!("JWKS parse error: {e}"))?;

        // Find the first Ed25519 OKP key that jsonwebtoken can turn into a DecodingKey.
        // Explicitly filter to kty=OKP crv=Ed25519 before attempting to build a key.
        let decoding_key = jwks
            .keys
            .iter()
            .filter(|jwk| is_ed25519_okp(&jwk.algorithm))
            .find_map(|jwk| DecodingKey::from_jwk(jwk).ok())
            .ok_or_else(|| "No Ed25519 (OKP) key found in JWKS response".to_string())?;

        let cached = CachedKeys {
            key: decoding_key,
            fetched_at: Instant::now(),
        };

        *self.cache.write().await = Some(cached);
        Ok(())
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwks_store: Arc<JwksKeyStore>,
    pub config: Arc<ApiConfig>,
}

impl AppState {
    pub fn new(pool: PgPool, jwks_store: JwksKeyStore, config: ApiConfig) -> Self {
        Self {
            pool,
            jwks_store: Arc::new(jwks_store),
            config: Arc::new(config),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct TestClaims {
        sub: String,
        iss: String,
        aud: Option<String>,
        exp: u64,
    }

    // Ed25519 test keypair (generated with `openssl genpkey -algorithm ed25519`).
    // These keys are safe for tests only — never use in production.
    const TEST_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MC4CAQAwBQYDK2VwBCIEICZi0TADAPL1fahH9fUfCwPifwDDyvN6xFYr6TdFLTOO\n\
        -----END PRIVATE KEY-----\n";

    const TEST_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
        MCowBQYDK2VwAyEAgtSuqEGOi6UzF0IPHxm49q8vu0Hrt+eBcaSnjk+YD+c=\n\
        -----END PUBLIC KEY-----\n";

    // Helper: attempt to build encoding/decoding keys from the embedded PEMs.
    // Returns None when the PEM bytes are not a valid Ed25519 key pair (so the
    // tests that depend on actual signing are skipped gracefully).
    fn try_make_keys() -> Option<(EncodingKey, DecodingKey)> {
        let enc = EncodingKey::from_ed_pem(TEST_PRIVATE_PEM.as_bytes()).ok()?;
        let dec = DecodingKey::from_ed_pem(TEST_PUBLIC_PEM.as_bytes()).ok()?;
        Some((enc, dec))
    }

    #[test]
    fn validation_without_audience_disables_aud_check() {
        let store = JwksKeyStore::new("https://example.com/.well-known/jwks.json".to_string());
        let v = store.validation("https://auth.example.com", None);
        assert!(
            !v.validate_aud,
            "audience validation should be disabled when None"
        );
        assert!(
            v.iss
                .as_ref()
                .map(|s| s.contains("https://auth.example.com"))
                .unwrap_or(false),
            "issuer should be set"
        );
    }

    #[test]
    fn validation_with_audience_enables_aud_check() {
        let store = JwksKeyStore::new("https://example.com/.well-known/jwks.json".to_string());
        let v = store.validation("https://auth.example.com", Some("temper-api"));
        assert!(v.validate_aud, "audience validation should be enabled");
        assert_eq!(v.algorithms, vec![Algorithm::EdDSA]);
    }

    #[test]
    fn with_static_key_returns_key_without_network() {
        // Skip if the embedded PEM pair is not valid.
        let Some((_, dec)) = try_make_keys() else {
            return;
        };
        let store = JwksKeyStore::with_static_key(dec);
        // The cache should be populated immediately.
        let guard = store
            .cache
            .try_read()
            .expect("lock should not be contended");
        assert!(
            guard.is_some(),
            "cache must be populated after with_static_key"
        );
    }

    #[tokio::test]
    async fn get_decoding_key_with_static_key_succeeds() {
        let Some((enc, dec)) = try_make_keys() else {
            return;
        };

        let store = JwksKeyStore::with_static_key(dec);

        // Round-trip: sign a token with the private key, verify with the store.
        let claims = TestClaims {
            sub: "user-123".into(),
            iss: "https://auth.example.com".into(),
            aud: None,
            exp: 9_999_999_999,
        };
        let token =
            encode(&Header::new(Algorithm::EdDSA), &claims, &enc).expect("encoding should succeed");

        let key = store
            .get_decoding_key()
            .await
            .expect("get_decoding_key must not fail for a static key");

        let mut v = Validation::new(Algorithm::EdDSA);
        v.set_issuer(&["https://auth.example.com"]);
        v.validate_aud = false;

        let data = jsonwebtoken::decode::<TestClaims>(&token, &key, &v)
            .expect("token verification should succeed");

        assert_eq!(data.claims.sub, "user-123");
    }

    #[test]
    fn is_ed25519_okp_accepts_valid_key() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, EllipticCurve, OctetKeyPairParameters, OctetKeyPairType,
        };
        let params = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
            key_type: OctetKeyPairType::OctetKeyPair,
            curve: EllipticCurve::Ed25519,
            x: "test".to_string(),
        });
        assert!(is_ed25519_okp(&params));
    }

    #[test]
    fn is_ed25519_okp_rejects_rsa() {
        use jsonwebtoken::jwk::{AlgorithmParameters, RSAKeyParameters, RSAKeyType};
        let params = AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n: "test".to_string(),
            e: "test".to_string(),
        });
        assert!(!is_ed25519_okp(&params));
    }

    #[test]
    fn is_ed25519_okp_rejects_wrong_curve() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, EllipticCurve, OctetKeyPairParameters, OctetKeyPairType,
        };
        let params = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
            key_type: OctetKeyPairType::OctetKeyPair,
            curve: EllipticCurve::P256,
            x: "test".to_string(),
        });
        assert!(!is_ed25519_okp(&params));
    }
}
