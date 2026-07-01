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
    /// The JWT algorithm matching the cached key's family (RS256 for RSA,
    /// EdDSA for Ed25519). Used to build a single-family validation allow-list.
    algorithm: Algorithm,
    fetched_at: Instant,
}

/// A verification key paired with the JWT algorithm matching its family.
///
/// `jsonwebtoken`'s `verify_signature` rejects any `Validation` whose allow-list
/// contains an algorithm from a different family than the key, so the algorithm
/// must travel with the key from the JWKS store to the `validation()` call.
#[derive(Clone)]
pub struct VerificationKey {
    pub key: DecodingKey,
    pub algorithm: Algorithm,
}

impl std::fmt::Debug for VerificationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerificationKey")
            .field("algorithm", &self.algorithm)
            .finish_non_exhaustive()
    }
}

/// Fetches RSA and EdDSA/Ed25519 public keys from a JWKS endpoint, caches them,
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

/// Map a JWK to the JWT algorithm it verifies: RSA → RS256, OKP/Ed25519 → EdDSA.
/// Returns `None` for unsupported key types.
fn algorithm_for_key(params: &AlgorithmParameters) -> Option<Algorithm> {
    match params {
        AlgorithmParameters::RSA(_) => Some(Algorithm::RS256),
        AlgorithmParameters::OctetKeyPair(p) if p.curve == EllipticCurve::Ed25519 => {
            Some(Algorithm::EdDSA)
        }
        _ => None,
    }
}

/// Check if a JWK is a supported key type: RSA (for RS256) or OKP/Ed25519 (for EdDSA).
/// Test-only: production maps the key to its algorithm directly via `algorithm_for_key`.
#[cfg(test)]
fn is_supported_key(params: &AlgorithmParameters) -> bool {
    algorithm_for_key(params).is_some()
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

    /// Create a key store pre-loaded with a static key and its algorithm.
    /// Intended for tests that do not have network access to a real JWKS endpoint.
    pub fn with_static_key(key: DecodingKey, algorithm: Algorithm) -> Self {
        let cached = CachedKeys {
            key,
            algorithm,
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

    /// Return the cached `VerificationKey` (key + its algorithm), refreshing from
    /// the JWKS endpoint if the cache is absent or has expired.
    pub async fn get_decoding_key(&self) -> Result<VerificationKey, String> {
        // Fast path: read lock, check freshness.
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < self.ttl {
                    return Ok(VerificationKey {
                        key: cached.key.clone(),
                        algorithm: cached.algorithm,
                    });
                }
            }
        }

        // Cache is stale or absent — refresh under write lock.
        self.refresh().await?;

        let guard = self.cache.read().await;
        guard
            .as_ref()
            .map(|c| VerificationKey {
                key: c.key.clone(),
                algorithm: c.algorithm,
            })
            .ok_or_else(|| "JWKS cache empty after refresh".to_string())
    }

    /// Build a `Validation` for the given issuer and optional audience, with an
    /// allow-list scoped to exactly `algorithm` (the loaded key's family).
    ///
    /// The allow-list must be single-family: `jsonwebtoken` rejects a
    /// `Validation` whose list mixes families the cached key does not match.
    pub fn validation(
        &self,
        issuer: &str,
        audience: Option<&str>,
        algorithm: Algorithm,
    ) -> Validation {
        let mut v = Validation::new(algorithm);
        v.algorithms = vec![algorithm];
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

        // Find the first supported key (RSA or Ed25519 OKP) that jsonwebtoken can
        // turn into a DecodingKey, capturing the algorithm matching its family.
        let (decoding_key, algorithm) = jwks
            .keys
            .iter()
            .find_map(|jwk| {
                let algorithm = algorithm_for_key(&jwk.algorithm)?;
                let key = DecodingKey::from_jwk(jwk).ok()?;
                Some((key, algorithm))
            })
            .ok_or_else(|| {
                "No supported key (RSA or Ed25519) found in JWKS response".to_string()
            })?;

        let cached = CachedKeys {
            key: decoding_key,
            algorithm,
            fetched_at: Instant::now(),
        };

        *self.cache.write().await = Some(cached);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AppState {
    pub pool: PgPool,
    pub jwks_store: Arc<JwksKeyStore>,
    pub config: Arc<ApiConfig>,
    /// OIDC userinfo endpoint, resolved once per process via discovery on the
    /// first email-fallback. Lazy (not boot-time) so there is no startup
    /// coupling to the IdP; shared across `AppState` clones via `Arc`.
    pub userinfo_endpoint: Arc<tokio::sync::OnceCell<String>>,
}

impl AppState {
    pub fn new(pool: PgPool, jwks_store: JwksKeyStore, config: ApiConfig) -> Self {
        Self {
            pool,
            jwks_store: Arc::new(jwks_store),
            config: Arc::new(config),
            userinfo_endpoint: Arc::new(tokio::sync::OnceCell::new()),
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
        let v = store.validation("https://auth.example.com", None, Algorithm::RS256);
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
        let v = store.validation(
            "https://auth.example.com",
            Some("temper-api"),
            Algorithm::RS256,
        );
        assert!(v.validate_aud, "audience validation should be enabled");
        assert!(v.algorithms.contains(&Algorithm::RS256));
    }

    #[test]
    fn with_static_key_returns_key_without_network() {
        // Skip if the embedded PEM pair is not valid.
        let Some((_, dec)) = try_make_keys() else {
            return;
        };
        let store = JwksKeyStore::with_static_key(dec, Algorithm::EdDSA);
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

        let store = JwksKeyStore::with_static_key(dec, Algorithm::EdDSA);

        // Round-trip: sign a token with the private key, verify with the store.
        let claims = TestClaims {
            sub: "user-123".into(),
            iss: "https://auth.example.com".into(),
            aud: None,
            exp: 9_999_999_999,
        };
        let token =
            encode(&Header::new(Algorithm::EdDSA), &claims, &enc).expect("encoding should succeed");

        let vk = store
            .get_decoding_key()
            .await
            .expect("get_decoding_key must not fail for a static key");
        assert_eq!(vk.algorithm, Algorithm::EdDSA);

        let mut v = Validation::new(Algorithm::EdDSA);
        v.set_issuer(&["https://auth.example.com"]);
        v.validate_aud = false;

        let data = jsonwebtoken::decode::<TestClaims>(&token, &vk.key, &v)
            .expect("token verification should succeed");

        assert_eq!(data.claims.sub, "user-123");
    }

    #[test]
    fn is_supported_key_accepts_ed25519() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, EllipticCurve, OctetKeyPairParameters, OctetKeyPairType,
        };
        let params = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
            key_type: OctetKeyPairType::OctetKeyPair,
            curve: EllipticCurve::Ed25519,
            x: "test".to_string(),
        });
        assert!(is_supported_key(&params));
    }

    #[test]
    fn is_supported_key_accepts_rsa() {
        use jsonwebtoken::jwk::{AlgorithmParameters, RSAKeyParameters, RSAKeyType};
        let params = AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n: "test".to_string(),
            e: "test".to_string(),
        });
        assert!(is_supported_key(&params));
    }

    #[test]
    fn is_rsa_key_accepted() {
        use jsonwebtoken::jwk::{AlgorithmParameters, RSAKeyParameters, RSAKeyType};
        let params = AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n: "test".to_string(),
            e: "test".to_string(),
        });
        assert!(is_supported_key(&params));
    }

    #[tokio::test]
    async fn validation_accepts_eddsa_token_for_eddsa_key() {
        let Some((enc, dec)) = try_make_keys() else {
            return;
        };

        let store = JwksKeyStore::with_static_key(dec.clone(), Algorithm::EdDSA);

        let claims = TestClaims {
            sub: "u1".into(),
            iss: "https://as.example".into(),
            aud: Some("https://api.example".into()),
            exp: 9_999_999_999,
        };
        let token =
            encode(&Header::new(Algorithm::EdDSA), &claims, &enc).expect("encoding should succeed");

        let vk = store
            .get_decoding_key()
            .await
            .expect("get_decoding_key must not fail for a static key");
        assert_eq!(vk.algorithm, Algorithm::EdDSA);

        let validation = store.validation(
            "https://as.example",
            Some("https://api.example"),
            Algorithm::EdDSA,
        );

        let data = jsonwebtoken::decode::<TestClaims>(&token, &vk.key, &validation)
            .expect("EdDSA token verification should succeed");
        assert_eq!(data.claims.sub, "u1");
    }

    #[test]
    fn is_supported_key_rejects_wrong_curve() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, EllipticCurve, OctetKeyPairParameters, OctetKeyPairType,
        };
        let params = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
            key_type: OctetKeyPairType::OctetKeyPair,
            curve: EllipticCurve::P256,
            x: "test".to_string(),
        });
        assert!(!is_supported_key(&params));
    }
}
