use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyOperations, PublicKeyUse,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::config::ApiConfig;

/// One verification key as published by the JWKS, carrying the `kid` that names it.
struct CachedKey {
    /// The JWKS entry's `kid`, or `None` for a document that publishes unnamed keys. A JWKS
    /// without `kid`s is legal, and the selection rule below stays compatible with it.
    kid: Option<String>,
    key: DecodingKey,
    /// The JWT algorithm matching this key's family (RS256 for RSA, EdDSA for Ed25519).
    /// Used to build a single-family validation allow-list.
    algorithm: Algorithm,
}

/// Cached keys with a timestamp for TTL-based invalidation.
///
/// Every supported key in the document is kept, not just the first. That is what makes an IdP
/// signing-key rollover a non-event: during the overlap the IdP publishes both keys and signs with
/// either, and both are here to be selected by `kid`.
struct CachedKeys {
    keys: Vec<CachedKey>,
    /// Whether the JWKS **document** named any of its keys — read from the raw document, not from
    /// `keys`.
    ///
    /// The distinction is the whole point. `keys` holds only what this store can verify with, and
    /// the entries it drops (an EC key, an encryption-only key) are often precisely the ones
    /// carrying a `kid`. Deciding "does this document name its keys?" from the survivors would let
    /// a document that names every key look unnamed, which silently turns `kid` pinning off and
    /// sends every token to the first key.
    document_named_keys: bool,
    fetched_at: Instant,
}

/// Pick the key a token names.
///
/// - Against a document that names its keys, a token naming a `kid` gets **that** key and nothing
///   else. There is deliberately no fall-back-to-the-first-key arm: `kid` is caller-supplied, and a
///   miss that quietly resolves to some other trusted key is how a rotation feature turns into a
///   verification defect.
/// - A token naming no `kid`, or any token against a document that names none of its keys, gets the
///   first key. An unnamed JWKS is legal and its key answers for every token.
///
/// `named` comes from the raw document rather than from `keys`, so dropping an entry this store
/// cannot verify with never changes which branch a token takes.
fn select_key<'a>(keys: &'a [CachedKey], named: bool, kid: Option<&str>) -> Option<&'a CachedKey> {
    match kid {
        Some(k) if named => keys.iter().find(|c| c.kid.as_deref() == Some(k)),
        _ => keys.first(),
    }
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

/// Why a token's verification key could not be produced.
///
/// The split is load-bearing at every call site. `UnknownKid` is a fact about the **token** — the
/// caller named a key this instance does not trust — and belongs wherever a bad token belongs: a
/// 401 that tells a client to authenticate again. `Unavailable` is a fact about the **JWKS
/// endpoint** and belongs on the 503 path.
///
/// Selecting a key by `kid` means a lookup can fail on input the *caller* chose, so the two classes
/// must stay apart: merged, anyone could mint a token with a nonsense `kid` and make a surface
/// answer that it is down.
#[derive(Debug)]
pub enum KeyLookupError {
    /// The token named a `kid` that no key in the JWKS carries, after a refresh.
    ///
    /// Construct with [`KeyLookupError::unknown_kid`] — the value is attacker-supplied and must be
    /// bounded before it reaches a log line.
    UnknownKid(String),
    /// The JWKS could not be fetched, or held no key this store can verify with.
    Unavailable(String),
}

/// How much of a token's `kid` is worth keeping for diagnostics.
///
/// A JWT header is caller-written and bounded only by the HTTP server's header buffer, so an
/// unbounded `kid` is a few hundred KB of attacker-chosen text per request — reaching the logs and,
/// through them, whatever ingests them. No diagnosis needs more than a prefix, and truncating at
/// construction bounds every consumer at once rather than asking each log site to remember.
const KID_DIAGNOSTIC_LEN: usize = 64;

impl KeyLookupError {
    /// Build an [`UnknownKid`](KeyLookupError::UnknownKid), truncated for diagnostics.
    pub fn unknown_kid(kid: &str) -> Self {
        let end = kid
            .char_indices()
            .nth(KID_DIAGNOSTIC_LEN)
            .map(|(i, _)| i)
            .unwrap_or(kid.len());
        Self::UnknownKid(kid[..end].to_owned())
    }
}

impl std::fmt::Display for KeyLookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKid(kid) => write!(f, "no key in the JWKS matches the token's kid {kid}"),
            Self::Unavailable(why) => write!(f, "{why}"),
        }
    }
}

/// Fetches RSA and EdDSA/Ed25519 public keys from a JWKS endpoint, caches them,
/// and provides them for JWT verification.
pub struct JwksKeyStore {
    url: String,
    client: reqwest::Client,
    cache: RwLock<Option<CachedKeys>>,
    ttl: Duration,
    /// Holds the instant of the last refresh triggered by a `kid` the cache could not resolve.
    ///
    /// Deliberately NOT the cache lock: a refresh must never block token verification, and the two
    /// guard different things. Holding this across the fetch is what makes the refresh
    /// single-flight — concurrent misses queue here, and all but the first find a recent stamp and
    /// return without fetching.
    unknown_kid_gate: tokio::sync::Mutex<Option<Instant>>,
    /// Collapses refreshes triggered by an absent or expired cache, and carries the outcome of the
    /// last such attempt.
    ///
    /// Separate from `unknown_kid_gate` because the two need **opposite** behaviour when contended,
    /// and merging them would silently give one of them the other's. On an unknown `kid` the cache
    /// still holds usable keys, so a waiter gains nothing and returns at once. Here the cache holds
    /// nothing this store may serve, so a waiter must take the in-flight fetch's result — returning
    /// early would either fail a request that was about to succeed, or serve a key the TTL has
    /// already retired.
    stale_refresh_gate: tokio::sync::Mutex<Option<(Instant, Result<(), String>)>>,
    /// How long a failed refresh answers for the requests behind it.
    ///
    /// Without this, single-flight alone turns a concurrent herd into a serial one against a
    /// failing endpoint: each waiter takes the lock, finds the cache still stale, and starts its
    /// own fetch, so the queue costs one full timeout per request. An endpoint that failed a moment
    /// ago has not recovered since, and saying so immediately is both truer and cheaper.
    failed_refresh_backoff: Duration,
    /// Floor on how often an unresolvable `kid` may trigger a fetch. `kid` is attacker-supplied, so
    /// without a floor anyone could turn one request into one JWKS fetch, indefinitely.
    ///
    /// The floor has a cost, stated so nobody has to rediscover it: a key published within one
    /// interval of the last miss-triggered fetch is refused until that fetch is allowed to happen.
    /// The delay is bounded by the interval and cannot be extended — the stamp only advances when
    /// a fetch actually runs, and that fetch is the one that picks the new key up.
    unknown_kid_refresh_interval: Duration,
}

impl std::fmt::Debug for JwksKeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwksKeyStore")
            .field("url", &self.url)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

/// The HTTP client used to fetch the JWKS.
///
/// Bounded deliberately. An endpoint that accepts the connection and never answers would otherwise
/// hold a request open indefinitely, and this fetch sits on the authentication path — every other
/// outbound call in this crate already carries a timeout.
fn jwks_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .build()
        // Only fails if the TLS backend cannot be initialised, which is not a runtime condition
        // this store can do anything about — and the default client is the same builder.
        .unwrap_or_else(|_| reqwest::Client::new())
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

/// Whether a JWK is offered for signature verification.
///
/// A JWKS is allowed to publish keys for purposes other than signing, and says so in `use` and
/// `key_ops` — Keycloak and Entra both publish an RSA encryption key beside the signing key.
/// `DecodingKey::from_jwk` consults neither field, so without this the store would trust every
/// RSA/Ed25519 entry in the document as a signer, including the ones the IdP has explicitly said
/// are not.
///
/// Absent fields mean yes: both are optional in RFC 7517, and the overwhelmingly common JWKS omits
/// them. Only an explicit statement to the contrary excludes a key.
fn is_offered_for_verification(jwk: &Jwk) -> bool {
    if matches!(jwk.common.public_key_use, Some(PublicKeyUse::Encryption)) {
        return false;
    }
    match &jwk.common.key_operations {
        Some(ops) => ops
            .iter()
            .any(|op| matches!(op, KeyOperations::Verify | KeyOperations::Sign)),
        None => true,
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
            client: jwks_http_client(),
            cache: RwLock::new(None),
            ttl: Duration::from_secs(3600),
            stale_refresh_gate: tokio::sync::Mutex::new(None),
            failed_refresh_backoff: Duration::from_secs(5),
            unknown_kid_gate: tokio::sync::Mutex::new(None),
            unknown_kid_refresh_interval: Duration::from_secs(60),
        }
    }

    /// Create a key store pre-loaded with a static key and its algorithm.
    /// Intended for tests that do not have network access to a real JWKS endpoint.
    pub fn with_static_key(key: DecodingKey, algorithm: Algorithm) -> Self {
        let cached = CachedKeys {
            // No `kid`: a static key answers for every token, which is what a test harness wants
            // and what `select_key`'s unnamed-document arm already does.
            keys: vec![CachedKey {
                kid: None,
                key,
                algorithm,
            }],
            document_named_keys: false,
            // Use a far-future instant so the cached key never expires.
            fetched_at: Instant::now() + Duration::from_secs(u32::MAX as u64),
        };
        Self {
            url: String::new(),
            client: jwks_http_client(),
            cache: RwLock::new(Some(cached)),
            // Very long TTL; the pre-loaded cache will never be refreshed.
            ttl: Duration::from_secs(u32::MAX as u64),
            stale_refresh_gate: tokio::sync::Mutex::new(None),
            failed_refresh_backoff: Duration::from_secs(5),
            unknown_kid_gate: tokio::sync::Mutex::new(None),
            unknown_kid_refresh_interval: Duration::from_secs(60),
        }
    }

    /// Return the `VerificationKey` (key + its algorithm) that `token` names in its `kid` header,
    /// refreshing from the JWKS endpoint if the cache is absent, expired, or does not hold that key.
    ///
    /// Takes the whole token rather than a `kid` so the header parse lives here once. Four call
    /// sites verify tokens; a `decode_header` copied into each of them is four chances to disagree
    /// about what an absent or malformed header means.
    pub async fn get_decoding_key_for_token(
        &self,
        token: &str,
    ) -> Result<VerificationKey, KeyLookupError> {
        // A header this store cannot parse is not this function's refusal to make: fall through
        // with no `kid` and let `decode` reject the token — it re-parses the same bytes and fails
        // on them identically, so nothing reaches a key on the strength of an unreadable header.
        let kid = jsonwebtoken::decode_header(token).ok().and_then(|h| h.kid);

        enum Next {
            Serve(VerificationKey),
            /// Fresh cache, but nothing in it answers for this `kid`. The key may have been
            /// published since the last fetch — this is the rotation case.
            UnknownKid,
            /// Absent or expired cache: there is nothing to select from until it is refreshed.
            Stale,
        }

        let next = {
            let guard = self.cache.read().await;
            match guard.as_ref() {
                Some(cached) if cached.fetched_at.elapsed() < self.ttl => {
                    match select_key(&cached.keys, cached.document_named_keys, kid.as_deref()) {
                        Some(c) => Next::Serve(VerificationKey {
                            key: c.key.clone(),
                            algorithm: c.algorithm,
                        }),
                        None => Next::UnknownKid,
                    }
                }
                _ => Next::Stale,
            }
        };

        match next {
            Next::Serve(vk) => return Ok(vk),
            // Nothing in the cache may be served, so this must refresh before it can answer, and a
            // failure is the endpoint's rather than the token's.
            Next::Stale => self
                .refresh_stale()
                .await
                .map_err(KeyLookupError::Unavailable)?,
            Next::UnknownKid => {
                // Best-effort. The cache still holds usable keys, so a JWKS blip must not be
                // reported as a fetch failure for a token whose `kid` may simply not exist.
                if let Err(e) = self.refresh_for_unknown_kid().await {
                    tracing::debug!("JWKS refresh after unknown kid failed: {e}");
                }
            }
        }

        let guard = self.cache.read().await;
        let cached = guard.as_ref().ok_or_else(|| {
            KeyLookupError::Unavailable("JWKS cache empty after refresh".to_string())
        })?;
        select_key(&cached.keys, cached.document_named_keys, kid.as_deref())
            .map(|c| VerificationKey {
                key: c.key.clone(),
                algorithm: c.algorithm,
            })
            .ok_or_else(|| match kid {
                Some(k) => KeyLookupError::unknown_kid(&k),
                // No `kid` and still nothing to select means the document itself is unusable, which
                // is the endpoint's problem rather than this token's.
                None => KeyLookupError::Unavailable("JWKS holds no supported key".to_string()),
            })
    }

    /// Refresh because the cache is absent or its TTL has lapsed.
    ///
    /// True single-flight: concurrent requests collapse onto **one** fetch and take its result,
    /// rather than each starting its own. Every request whose cache is stale reaches this — a cold
    /// process answers its first burst here, and so does every instance of a rolling deploy — so
    /// the fan-out is one outbound fetch per inbound request unless something collapses it.
    ///
    /// Waiting on the lock is correct here, unlike in [`Self::refresh_for_unknown_kid`]: a waiter
    /// has nothing it could serve instead, and one shared fetch is strictly better for it than its
    /// own. The wait is bounded by the client's timeout, and the fast path — a fresh cache that
    /// answers the token — never reaches this function at all.
    async fn refresh_stale(&self) -> Result<(), String> {
        let mut gate = self.stale_refresh_gate.lock().await;

        // A flight that finished while we waited for the lock has already done this work.
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < self.ttl {
                    return Ok(());
                }
            }
        }

        // ...or it may have just failed, in which case its answer is ours too.
        if let Some((at, Err(why))) = gate.as_ref() {
            if at.elapsed() < self.failed_refresh_backoff {
                return Err(why.clone());
            }
        }

        let result = self.refresh().await;
        *gate = Some((Instant::now(), result.clone()));
        result
    }

    /// Refresh because a token named a `kid` the cache could not resolve.
    ///
    /// Single-flight and rate-limited. Since `kid` is chosen by whoever minted the token, an
    /// unbounded version of this would be a free amplifier aimed at the IdP.
    ///
    /// **`try_lock`, never `lock`.** A contended gate means a fetch is already in flight, and this
    /// request has nothing to gain by waiting for it: it would re-select against a cache the
    /// in-flight fetch has not written yet, and meanwhile it is holding a connection and a task
    /// open. Waiting also puts the rate-limit check *behind* the network call, so the requests the
    /// check exists to answer cheaply are exactly the ones that would queue on it — and with a
    /// slow JWKS the queue is bounded only by the fetch, which is the pile-up this must not build.
    /// Returning immediately costs the caller one refused token and nothing else.
    async fn refresh_for_unknown_kid(&self) -> Result<(), String> {
        let Ok(mut gate) = self.unknown_kid_gate.try_lock() else {
            return Ok(());
        };
        if let Some(last) = *gate {
            if last.elapsed() < self.unknown_kid_refresh_interval {
                return Ok(());
            }
        }
        // Stamped before the fetch, so a FAILING endpoint is rate-limited exactly as a succeeding
        // one is. The bound is on attempts, not on successes.
        *gate = Some(Instant::now());
        self.refresh().await
    }

    /// The tolerance applied to the `exp`/`nbf` window, in seconds: a token is accepted up to
    /// this long past `exp` (and up to this long before `nbf`).
    ///
    /// **A security-relevant value is stated here rather than inherited.** `jsonwebtoken`'s own
    /// default is also 60 seconds, which is exactly why the explicit assignment matters: while the
    /// two agree, the library default and our choice are indistinguishable, and a library release
    /// that moved its default would silently move ours with it. The pin keeps the tolerance at the
    /// value this codebase chose.
    const CLOCK_SKEW_LEEWAY_SECONDS: u64 = 60;

    /// Build a `Validation` for the given issuer and audience, with an allow-list scoped to exactly
    /// `algorithm` (the loaded key's family).
    ///
    /// The allow-list must be single-family: `jsonwebtoken` rejects a `Validation` whose list mixes
    /// families the cached key does not match.
    ///
    /// **Two separate things must be true for the audience to actually be checked**, and only the
    /// first is obvious:
    ///
    /// 1. The config must carry at least one audience. It used to be an `Option`, and a `None` set
    ///    `validate_aud = false` — so an unset `AUTH_AUDIENCE` disabled the check outright. There is
    ///    no `None` left to hand in (see [`crate::auth_config`]).
    /// 2. **The token must actually carry an `aud` claim.** `set_audience` alone does NOT enforce
    ///    that. `required_spec_claims` defaults to `{"exp"}`, and jsonwebtoken's own docs say:
    ///    *"Validation only happens if `aud` claim is present in the token."* A token with **no**
    ///    `aud` — or a malformed one that fails to parse as a string — hits the fallthrough arm and
    ///    is **accepted**. Fixing (1) without (2) closes half the door and documents the other half
    ///    as shut.
    /// 3. The audience set is a SET, not a single value: the MCP surface passes its own RFC 8707
    ///    resource audience *plus* the API audience, because machine tokens and sessions minted
    ///    before `MCP_AUDIENCE` existed carry the API audience and both surfaces are one instance.
    ///    A token naming EITHER audience verifies; one naming neither does not.
    ///
    /// `iss` is required for the same reason: the issuer match has the same present-only semantics,
    /// so a claims-minimal token signed by any key in the trusted JWKS would otherwise walk in.
    pub fn validation(&self, issuer: &str, audiences: &[&str], algorithm: Algorithm) -> Validation {
        let mut v = Validation::new(algorithm);
        v.algorithms = vec![algorithm];
        v.leeway = Self::CLOCK_SKEW_LEEWAY_SECONDS;
        v.set_required_spec_claims(&["exp", "iss", "aud"]);
        v.set_issuer(&[issuer]);
        v.set_audience(audiences);
        v
    }

    /// Fetch the JWKS endpoint, parse **every** usable RSA/OKP-Ed25519 key with the `kid` that
    /// names it, and store them in the cache.
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

        // Keep EVERY supported key (RSA or Ed25519 OKP) that jsonwebtoken can turn into a
        // DecodingKey, each with the algorithm matching its family and the `kid` that names it.
        // Entries this store cannot verify with are skipped rather than failing the document: a
        // JWKS is allowed to publish key types it does not support, and dropping the whole document
        // over one would take down verification for the keys it does support. Note the limit of
        // that tolerance — skipping happens AFTER `serde` has parsed the document, so an entry
        // whose `alg` or `kty` will not deserialize still fails the whole fetch.
        let keys: Vec<CachedKey> = jwks
            .keys
            .iter()
            .filter(|jwk| is_offered_for_verification(jwk))
            .filter_map(|jwk| {
                let algorithm = algorithm_for_key(&jwk.algorithm)?;
                let key = DecodingKey::from_jwk(jwk).ok()?;
                Some(CachedKey {
                    kid: jwk.common.key_id.clone(),
                    key,
                    algorithm,
                })
            })
            .collect();

        if keys.is_empty() {
            return Err("No supported key (RSA or Ed25519) found in JWKS response".to_string());
        }

        let cached = CachedKeys {
            keys,
            document_named_keys: jwks.keys.iter().any(|j| j.common.key_id.is_some()),
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
    /// The credential broker — temper's outbound reach to remote systems. Resolved
    /// from config: the Vercel Connect adapter when configured, else a
    /// `NullBroker` that fails mints clearly. Surfaces dispatch through it (the
    /// connection attach path mints once to verify).
    pub broker: Arc<dyn crate::broker::CredentialBroker>,
}

impl AppState {
    pub fn new(pool: PgPool, jwks_store: JwksKeyStore, config: ApiConfig) -> Self {
        let broker = crate::broker::resolve_broker(config.vercel_connect.clone());
        Self {
            pool,
            jwks_store: Arc::new(jwks_store),
            config: Arc::new(config),
            userinfo_endpoint: Arc::new(tokio::sync::OnceCell::new()),
            broker,
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
    #[rustfmt::skip]
    const TEST_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEICZi0TADAPL1fahH9fUfCwPifwDDyvN6xFYr6TdFLTOO\n-----END PRIVATE KEY-----\n"; // gitleaks:allow — inline test keypair, no production trust relationship

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

    // A SECOND Ed25519 test keypair, so a JWKS can publish two keys and a test can prove the store
    // picks the one the token names rather than the one that happens to be first.
    // Test-only, same as the pair above.
    #[rustfmt::skip]
    const TEST_PRIVATE_PEM_2: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIMoN2GZ+oOtD7JwVGY6jLSqAnfaSIDSeg31tntarak9e\n-----END PRIVATE KEY-----\n"; // gitleaks:allow — inline test keypair, no production trust relationship

    const TEST_PUBLIC_PEM_2: &str = "-----BEGIN PUBLIC KEY-----\n\
        MCowBQYDK2VwAyEAzo3nINJTwVoMArx1c/sxgHK6s+Plqvzb0Rh6Hj65Hg4=\n\
        -----END PUBLIC KEY-----\n";

    /// The `x` parameter of an Ed25519 JWK: the raw 32-byte public key, base64url, unpadded.
    /// An Ed25519 SPKI DER is a 12-byte prefix followed by exactly those 32 bytes.
    fn jwk_x_from_public_pem(pem: &str) -> String {
        use base64::Engine as _;
        let b64: String = pem
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect::<String>();
        let der = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .expect("public PEM body is base64");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&der[der.len() - 32..])
    }

    fn ed25519_jwk(kid: &str, public_pem: &str) -> serde_json::Value {
        serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "kid": kid,
            "x": jwk_x_from_public_pem(public_pem),
        })
    }

    fn sign_with(kid: Option<&str>, private_pem: &str) -> String {
        let enc = EncodingKey::from_ed_pem(private_pem.as_bytes()).expect("valid Ed25519 PKCS#8");
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = kid.map(str::to_owned);
        let claims = TestClaims {
            sub: "rotating-user".into(),
            iss: "https://as.example".into(),
            aud: Some("https://api.example".into()),
            exp: 9_999_999_999,
        };
        encode(&header, &claims, &enc).expect("encoding should succeed")
    }

    /// Stand up a JWKS endpoint serving exactly `jwks`, mounted so a later `mount_as_scoped` can
    /// replace it — that is how a test publishes a rotated document mid-run.
    async fn jwks_server(keys: Vec<serde_json::Value>) -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        publish_jwks(&server, keys).await;
        server
    }

    async fn publish_jwks(server: &wiremock::MockServer, keys: Vec<serde_json::Value>) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "keys": keys })),
            )
            .mount(server)
            .await;
    }

    fn verify(vk: &VerificationKey, store: &JwksKeyStore, token: &str) -> Result<String, String> {
        let validation =
            store.validation("https://as.example", &["https://api.example"], vk.algorithm);
        jsonwebtoken::decode::<TestClaims>(token, &vk.key, &validation)
            .map(|d| d.claims.sub)
            .map_err(|e| e.to_string())
    }

    /// Selection is by `kid`, not by position: a token names the key that signed it, and that is the
    /// key the store returns. A JWKS may publish several keys at once — that is what an IdP does
    /// during a rollover — and every published key must be selectable.
    #[tokio::test]
    async fn a_token_signed_by_the_second_key_in_the_jwks_verifies() {
        let server = jwks_server(vec![
            ed25519_jwk("key-1", TEST_PUBLIC_PEM),
            ed25519_jwk("key-2", TEST_PUBLIC_PEM_2),
        ])
        .await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let token = sign_with(Some("key-2"), TEST_PRIVATE_PEM_2);
        let vk = store
            .get_decoding_key_for_token(&token)
            .await
            .expect("the second key is published and named by the token");

        assert_eq!(
            verify(&vk, &store, &token).expect("verifies"),
            "rotating-user"
        );
    }

    /// The first key still works — selection must not have simply moved the off-by-one.
    #[tokio::test]
    async fn a_token_signed_by_the_first_key_in_a_two_key_jwks_still_verifies() {
        let server = jwks_server(vec![
            ed25519_jwk("key-1", TEST_PUBLIC_PEM),
            ed25519_jwk("key-2", TEST_PUBLIC_PEM_2),
        ])
        .await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let token = sign_with(Some("key-1"), TEST_PRIVATE_PEM);
        let vk = store
            .get_decoding_key_for_token(&token)
            .await
            .expect("key-1 is published");

        assert_eq!(
            verify(&vk, &store, &token).expect("verifies"),
            "rotating-user"
        );
    }

    /// The availability half: a key published AFTER the last fetch must not wait out the TTL. The
    /// store's TTL is an hour, so a test that passes here cannot have been served by expiry.
    #[tokio::test]
    async fn a_newly_published_key_verifies_without_waiting_out_the_ttl() {
        let server = jwks_server(vec![ed25519_jwk("key-1", TEST_PUBLIC_PEM)]).await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        // Warm the cache on the old document, so the miss below is against a FRESH cache.
        let old = sign_with(Some("key-1"), TEST_PRIVATE_PEM);
        store
            .get_decoding_key_for_token(&old)
            .await
            .expect("key-1 loads");
        assert!(
            store.ttl >= Duration::from_secs(3600),
            "the TTL must be long enough that expiry cannot explain a pass"
        );

        // The IdP rotates: a second key appears in the document.
        publish_jwks(
            &server,
            vec![
                ed25519_jwk("key-1", TEST_PUBLIC_PEM),
                ed25519_jwk("key-2", TEST_PUBLIC_PEM_2),
            ],
        )
        .await;

        let new = sign_with(Some("key-2"), TEST_PRIVATE_PEM_2);
        let vk = store
            .get_decoding_key_for_token(&new)
            .await
            .expect("an unknown kid must trigger a refresh, not an hour of 401s");

        assert_eq!(
            verify(&vk, &store, &new).expect("verifies"),
            "rotating-user"
        );
    }

    /// Bounded: a kid that will never resolve must not buy one JWKS fetch per request. Anyone can
    /// mint a token with an arbitrary `kid` header, so an unbounded refresh is a free amplifier
    /// pointed at the IdP.
    #[tokio::test]
    async fn an_unknown_kid_refreshes_at_most_once_per_interval() {
        let server = jwks_server(vec![ed25519_jwk("key-1", TEST_PUBLIC_PEM)]).await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let known = sign_with(Some("key-1"), TEST_PRIVATE_PEM);
        store
            .get_decoding_key_for_token(&known)
            .await
            .expect("warms the cache");
        let after_warmup = server.received_requests().await.expect("recording").len();

        let bogus = sign_with(Some("no-such-key"), TEST_PRIVATE_PEM);
        for _ in 0..5 {
            assert!(
                store.get_decoding_key_for_token(&bogus).await.is_err(),
                "an unknown kid is refused, not quietly served the first key"
            );
        }

        let fetches = server.received_requests().await.expect("recording").len() - after_warmup;
        assert_eq!(fetches, 1, "five misses must buy one refresh, not five");
    }

    /// Single-flight: concurrent misses collapse to one fetch rather than a thundering herd.
    #[tokio::test]
    async fn concurrent_unknown_kid_misses_collapse_to_one_refresh() {
        let server = jwks_server(vec![ed25519_jwk("key-1", TEST_PUBLIC_PEM)]).await;
        let store = Arc::new(JwksKeyStore::new(format!(
            "{}/.well-known/jwks.json",
            server.uri()
        )));

        let known = sign_with(Some("key-1"), TEST_PRIVATE_PEM);
        store
            .get_decoding_key_for_token(&known)
            .await
            .expect("warms the cache");
        let after_warmup = server.received_requests().await.expect("recording").len();

        let bogus = sign_with(Some("no-such-key"), TEST_PRIVATE_PEM);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = Arc::clone(&store);
            let token = bogus.clone();
            handles.push(tokio::spawn(async move {
                store.get_decoding_key_for_token(&token).await.is_err()
            }));
        }
        for h in handles {
            assert!(h.await.expect("task"), "every concurrent miss is refused");
        }

        let fetches = server.received_requests().await.expect("recording").len() - after_warmup;
        assert_eq!(fetches, 1, "eight concurrent misses must buy one refresh");
    }

    /// A JWKS that publishes no `kid` at all is legal, and its key answers for every token —
    /// including one that names a `kid`, since the document offers nothing to match it against.
    #[tokio::test]
    async fn a_jwks_with_no_kids_still_serves_its_only_key() {
        let server = jwks_server(vec![serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": jwk_x_from_public_pem(TEST_PUBLIC_PEM),
        })])
        .await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        // Both a token that names a kid and one that does not must resolve, because there is no
        // kid in the document to disagree with.
        for token in [
            sign_with(None, TEST_PRIVATE_PEM),
            sign_with(Some("whatever"), TEST_PRIVATE_PEM),
        ] {
            let vk = store
                .get_decoding_key_for_token(&token)
                .await
                .expect("the only key loads");
            assert_eq!(
                verify(&vk, &store, &token).expect("verifies"),
                "rotating-user"
            );
        }
    }

    /// Fail-closed, and the one that must not regress: once the document DOES name its keys, a token
    /// naming a key that is not in it is refused. It is not served the first key as a fallback.
    #[tokio::test]
    async fn an_unknown_kid_is_refused_rather_than_falling_back_to_the_first_key() {
        let server = jwks_server(vec![ed25519_jwk("key-1", TEST_PUBLIC_PEM)]).await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let err = store
            .get_decoding_key_for_token(&sign_with(Some("attacker-chosen"), TEST_PRIVATE_PEM))
            .await
            .expect_err("a kid the JWKS does not publish resolves to no key");
        assert!(
            matches!(&err, KeyLookupError::UnknownKid(k) if k == "attacker-chosen"),
            "the refusal names the kid it could not resolve: {err:?}"
        );
    }

    /// Serve the JWKS with a delay, so a concurrent test is actually concurrent: without one the
    /// first fetch can finish before the others reach the gate, and the test would pass on a store
    /// that collapses nothing.
    async fn slow_jwks_server(
        keys: Vec<serde_json::Value>,
        delay: Duration,
    ) -> wiremock::MockServer {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "keys": keys }))
                    .set_delay(delay),
            )
            .mount(&server)
            .await;
        server
    }

    /// Every request whose cache is cold or expired must fetch before it can answer anything, so
    /// without single-flight the fan-out is one outbound fetch per inbound request — at process
    /// start, on every instance of a rolling deploy, and at each TTL boundary.
    #[tokio::test]
    async fn a_cold_cache_under_load_fetches_the_jwks_once() {
        let server = slow_jwks_server(
            vec![ed25519_jwk("key-1", TEST_PUBLIC_PEM)],
            Duration::from_millis(150),
        )
        .await;
        let store = Arc::new(JwksKeyStore::new(format!(
            "{}/.well-known/jwks.json",
            server.uri()
        )));

        let token = sign_with(Some("key-1"), TEST_PRIVATE_PEM);
        let mut handles = Vec::new();
        for _ in 0..32 {
            let store = Arc::clone(&store);
            let token = token.clone();
            handles.push(tokio::spawn(async move {
                store.get_decoding_key_for_token(&token).await.is_ok()
            }));
        }
        for h in handles {
            // Every waiter takes the shared fetch's RESULT — collapsing must not cost anyone their
            // answer, which is what separates this from the unknown-kid gate's early return.
            assert!(
                h.await.expect("task"),
                "a collapsed request still gets its key"
            );
        }

        let fetches = server.received_requests().await.expect("recording").len();
        assert_eq!(
            fetches, 1,
            "32 concurrent cold-cache requests must buy one fetch"
        );
    }

    /// The failure direction. Single-flight alone would turn the herd serial rather than removing
    /// it: each waiter finds the cache still stale and starts its own fetch, so a failing endpoint
    /// costs one full timeout per queued request.
    #[tokio::test]
    async fn a_failing_jwks_answers_the_whole_queue_from_one_attempt() {
        // A server with no mount answers 404, so `refresh` fails.
        let server = wiremock::MockServer::start().await;
        let store = Arc::new(JwksKeyStore::new(format!(
            "{}/.well-known/jwks.json",
            server.uri()
        )));

        let token = sign_with(Some("key-1"), TEST_PRIVATE_PEM);
        for _ in 0..8 {
            let err = store
                .get_decoding_key_for_token(&token)
                .await
                .expect_err("nothing is published");
            // The refusal stays the endpoint's, never the token's — a caller must not be told its
            // token is bad because the JWKS is down.
            assert!(matches!(err, KeyLookupError::Unavailable(_)), "{err:?}");
        }

        let fetches = server.received_requests().await.expect("recording").len();
        assert_eq!(
            fetches, 1,
            "a recent failure answers for the requests behind it"
        );
    }

    /// Collapsing must never be achieved by serving what the TTL retired. A key that has expired is
    /// refetched, and a document that changed underneath is picked up.
    #[tokio::test]
    async fn an_expired_cache_is_refetched_rather_than_served() {
        let server = jwks_server(vec![ed25519_jwk("key-1", TEST_PUBLIC_PEM)]).await;
        let mut store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));
        // A TTL of zero makes every read stale, which is the condition under test.
        store.ttl = Duration::ZERO;

        let token = sign_with(Some("key-1"), TEST_PRIVATE_PEM);
        store
            .get_decoding_key_for_token(&token)
            .await
            .expect("first read fetches");
        store
            .get_decoding_key_for_token(&token)
            .await
            .expect("second read refetches");

        assert_eq!(
            server.received_requests().await.expect("recording").len(),
            2,
            "an expired cache must not be served as if it were fresh"
        );
    }

    /// A JWKS says which of its keys are for verifying signatures, in `use` and `key_ops`.
    /// `DecodingKey::from_jwk` consults neither, so the store has to. Keycloak and Entra both
    /// publish an encryption key beside the signing key, and a key the IdP has said is not for
    /// verification must not become a trusted signer by being in the same document.
    #[tokio::test]
    async fn a_key_the_idp_publishes_for_encryption_is_not_a_trusted_signer() {
        let mut enc = ed25519_jwk("enc-1", TEST_PUBLIC_PEM_2);
        enc["use"] = serde_json::json!("enc");
        let server = jwks_server(vec![ed25519_jwk("sig-1", TEST_PUBLIC_PEM), enc]).await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let err = store
            .get_decoding_key_for_token(&sign_with(Some("enc-1"), TEST_PRIVATE_PEM_2))
            .await
            .expect_err("an encryption key must not verify a token");
        assert!(matches!(err, KeyLookupError::UnknownKid(_)), "{err:?}");

        // The signing key beside it is unaffected.
        let token = sign_with(Some("sig-1"), TEST_PRIVATE_PEM);
        let vk = store
            .get_decoding_key_for_token(&token)
            .await
            .expect("sig-1 loads");
        assert_eq!(
            verify(&vk, &store, &token).expect("verifies"),
            "rotating-user"
        );
    }

    /// Same, stated through `key_ops` instead of `use`.
    #[tokio::test]
    async fn a_key_whose_key_ops_exclude_verification_is_not_a_trusted_signer() {
        let mut derive = ed25519_jwk("derive-1", TEST_PUBLIC_PEM_2);
        derive["key_ops"] = serde_json::json!(["deriveKey"]);
        let server = jwks_server(vec![ed25519_jwk("sig-1", TEST_PUBLIC_PEM), derive]).await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let err = store
            .get_decoding_key_for_token(&sign_with(Some("derive-1"), TEST_PRIVATE_PEM_2))
            .await
            .expect_err("a derive-only key must not verify a token");
        assert!(matches!(err, KeyLookupError::UnknownKid(_)), "{err:?}");
    }

    /// Whether `kid` pinning applies is a property of the DOCUMENT, not of the keys that survived
    /// filtering. A document that names its keys but whose named entries are ones this store cannot
    /// verify with must not read as unnamed — that would switch pinning off silently and send every
    /// token to the first key, on a JWKS shape the operator does not control.
    #[tokio::test]
    async fn a_named_document_keeps_kid_pinning_even_when_the_named_entries_are_unsupported() {
        // An EC key — named, and skipped by `algorithm_for_key` — beside an UNNAMED Ed25519 key.
        let ec = serde_json::json!({
            "kty": "EC", "crv": "P-256", "kid": "ec-1",
            "x": "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU",
            "y": "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0",
        });
        let unnamed = serde_json::json!({
            "kty": "OKP", "crv": "Ed25519", "x": jwk_x_from_public_pem(TEST_PUBLIC_PEM),
        });
        let server = jwks_server(vec![ec, unnamed]).await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let err = store
            .get_decoding_key_for_token(&sign_with(Some("invented"), TEST_PRIVATE_PEM))
            .await
            .expect_err("the document names its keys, so an unmatched kid resolves to nothing");
        assert!(matches!(err, KeyLookupError::UnknownKid(_)), "{err:?}");
    }

    /// `kid` is chosen by whoever minted the token, so an unresolvable one must be classified as a
    /// fault of the TOKEN. Were it `Unavailable`, the MCP surface would answer 503 — telling the
    /// client to come back later instead of to authenticate again, which is exactly what a client
    /// whose key has just rotated must not be told, and which anyone could trigger at will.
    #[tokio::test]
    async fn an_unresolvable_kid_is_a_token_fault_and_a_dead_endpoint_is_an_outage() {
        let server = jwks_server(vec![ed25519_jwk("key-1", TEST_PUBLIC_PEM)]).await;
        let store = JwksKeyStore::new(format!("{}/.well-known/jwks.json", server.uri()));

        let by_token = store
            .get_decoding_key_for_token(&sign_with(Some("nope"), TEST_PRIVATE_PEM))
            .await
            .expect_err("unresolvable kid");
        assert!(
            matches!(by_token, KeyLookupError::UnknownKid(_)),
            "a kid the caller invented is not an outage: {by_token:?}"
        );

        // Nothing answering at the URL is the other class, and must stay distinguishable.
        let dead = JwksKeyStore::new("http://127.0.0.1:1/.well-known/jwks.json".to_string());
        let by_endpoint = dead
            .get_decoding_key_for_token(&sign_with(Some("key-1"), TEST_PRIVATE_PEM))
            .await
            .expect_err("nothing is listening");
        assert!(
            matches!(by_endpoint, KeyLookupError::Unavailable(_)),
            "an unreachable JWKS is an outage, not a bad token: {by_endpoint:?}"
        );
    }

    #[test]
    fn validation_always_enables_the_aud_check() {
        let store = JwksKeyStore::new("https://example.com/.well-known/jwks.json".to_string());
        let v = store.validation(
            "https://auth.example.com",
            &["temper-api"],
            Algorithm::RS256,
        );
        assert!(
            v.validate_aud,
            "audience validation must never be disabled — the caller cannot express 'no audience'"
        );
        assert!(v.algorithms.contains(&Algorithm::RS256));
        assert!(
            v.iss
                .as_ref()
                .map(|s| s.contains("https://auth.example.com"))
                .unwrap_or(false),
            "issuer should be set"
        );
    }

    /// The exp/`nbf` tolerance is the stated constant, not whatever `jsonwebtoken` defaults to.
    ///
    /// Today the constant equals the library's default (60s), so this test cannot tell the two
    /// apart — that indistinguishability is why the pin exists. If the library ever moves its
    /// default, this is what keeps our tolerance at the value we chose instead of drifting with it.
    #[test]
    fn the_clock_skew_tolerance_is_the_stated_value_not_the_library_default() {
        let store = JwksKeyStore::new("https://example.com/.well-known/jwks.json".to_string());
        let v = store.validation(
            "https://auth.example.com",
            &["temper-api"],
            Algorithm::RS256,
        );
        assert_eq!(
            v.leeway,
            JwksKeyStore::CLOCK_SKEW_LEEWAY_SECONDS,
            "the leeway must be the explicit constant, never an inherited default"
        );
        assert_eq!(
            v.leeway,
            60,
            "jsonwebtoken 9.3.1's default is also 60 — move this literal only as a deliberate posture change"
        );
    }

    /// `set_audience` is NOT sufficient, and this is the trap: jsonwebtoken only checks `aud` when
    /// the claim is PRESENT (`required_spec_claims` defaults to `{"exp"}`). A token carrying no
    /// `aud` at all was accepted even with `validate_aud = true`.
    #[test]
    fn aud_and_iss_are_required_to_be_present_not_merely_matched() {
        let store = JwksKeyStore::new("https://example.com/.well-known/jwks.json".to_string());
        let v = store.validation(
            "https://auth.example.com",
            &["temper-api"],
            Algorithm::RS256,
        );

        assert!(
            v.required_spec_claims.contains("aud"),
            "a token with NO aud claim must be refused, not silently accepted"
        );
        assert!(
            v.required_spec_claims.contains("iss"),
            "the issuer match has the same present-only semantics as the audience match"
        );
        assert!(v.required_spec_claims.contains("exp"));
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
            .get_decoding_key_for_token(&token)
            .await
            .expect("key lookup must not fail for a static key");
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
            .get_decoding_key_for_token(&token)
            .await
            .expect("key lookup must not fail for a static key");
        assert_eq!(vk.algorithm, Algorithm::EdDSA);

        let validation = store.validation(
            "https://as.example",
            &["https://api.example"],
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
