//! The real blob provider client: Vercel Blob, private storage (spec: binary blobs, D6).
//!
//! Wire protocol grounded in `@vercel/blob` 2.8.0 source (dist source maps, extracted
//! 2026-09-01): API base `https://vercel.com/api/blob` with `x-api-version: 12`; bearer auth
//! where the token is the OIDC token or the read-write token; `x-vercel-blob-store-id` carries
//! the bare store id on every API call because the store id is not encoded in the OIDC token.
//! Reads do NOT go through the API: `GET https://<store>.private.blob.vercel-storage.com/<pathname>`
//! with the same bearer — the provider authorizes the store host directly and streams, which is
//! what lets our read-through stream rather than buffer (D6: streamed function responses carry
//! no platform body cap; buffered ones are hard-capped at 4.5 MB).
//!
//! This module never parses blob bytes: identity is the content-addressed pathname, decided
//! upstream (`blob_pathname`), and the ledger's hash-not-bytes invariant (D4) means the client
//! has no hash to compute and none to trust.

use anyhow::{Context, Result};
use bytes::Bytes;
use std::time::Duration;
use temper_substrate::blob_store::{BlobHead, BlobStore, PutReceipt};

use crate::config::BlobConfig;

/// The provider API version the wire shapes above are grounded against.
const BLOB_API_VERSION: &str = "12";
const DEFAULT_API_BASE: &str = "https://vercel.com/api/blob";

/// The provider HTTP posture is bounded: `connect_timeout` covers dialing, and
/// `read_timeout` is an idle-read bound — it resets on every chunk received, so the
/// streamed read (D6) survives a slow-but-flowing provider while a truly stalled
/// connection fails inside the window instead of hanging a blob door forever. There
/// is deliberately NO total timeout: the streamed get is unbounded by design, and a
/// stalled request WRITE (the mirror case) stays unbounded with it — the declared
/// tradeoff. Redirects are pinned to a small explicit bound; reqwest's implicit
/// follow-up-to-10 is nobody's decision.
const PROVIDER_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PROVIDER_READ_TIMEOUT: Duration = Duration::from_secs(30);
const PROVIDER_REDIRECT_LIMIT: usize = 3;

fn http_client(connect_timeout: Duration, read_timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .read_timeout(read_timeout)
        .redirect(reqwest::redirect::Policy::limited(PROVIDER_REDIRECT_LIMIT))
        .build()
        .expect("provider client configuration is static and valid")
}

pub struct VercelBlobStore {
    http: reqwest::Client,
    config: BlobConfig,
    api_base: String,
    read_host_base: String,
}

// Delegates to BlobConfig's redacting Debug — the read-write token never prints.
impl std::fmt::Debug for VercelBlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VercelBlobStore")
            .field("config", &self.config)
            .finish()
    }
}

impl VercelBlobStore {
    pub fn new(config: BlobConfig) -> Self {
        Self {
            http: http_client(PROVIDER_CONNECT_TIMEOUT, PROVIDER_READ_TIMEOUT),
            config,
            api_base: std::env::var("VERCEL_BLOB_API_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_API_BASE.to_string()),
            read_host_base: String::new(),
        }
    }

    /// Point both provider hosts at one base URL (a local mock server). Test machinery:
    /// production resolves the hosts from the store id, as the provider requires.
    #[doc(hidden)]
    pub fn with_test_endpoints(mut self, base_url: &str) -> Self {
        self.api_base = format!("{base_url}/api/blob");
        self.read_host_base = base_url.to_string();
        self
    }

    /// Swap in a client with short timeouts, so a witness can hold a stalled provider
    /// and assert the door fails bounded instead of hanging. Test machinery, the same
    /// tier as `with_test_endpoints`.
    #[doc(hidden)]
    pub fn with_test_timeouts(mut self, connect: Duration, read: Duration) -> Self {
        self.http = http_client(connect, read);
        self
    }

    /// The bearer token for one call — OIDC re-read per request (it rotates; caching one
    /// outlives it), static token as fallback.
    fn bearer(&self) -> Result<String> {
        self.config.bearer_token()
    }

    /// The content-addressed read URL: `https://<store>.private.blob.vercel-storage.com/<pathname>`.
    fn read_url(&self, pathname: &str) -> String {
        if !self.read_host_base.is_empty() {
            return format!("{}/{pathname}", self.read_host_base);
        }
        format!(
            "https://{}.private.blob.vercel-storage.com/{pathname}",
            self.config.store_id
        )
    }

    fn api_url(&self, path: &str, query: &str) -> String {
        let sep = if query.is_empty() { "" } else { "?" };
        format!("{}{path}{sep}{query}", self.api_base)
    }

    /// The headers every API call carries (grounded: `requestApi` in the SDK's helpers.ts).
    fn api_headers(&self, bearer: &str) -> Result<reqwest::header::HeaderMap> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("authorization", format!("Bearer {bearer}").parse()?);
        headers.insert("x-vercel-blob-store-id", self.config.store_id.parse()?);
        headers.insert("x-api-version", BLOB_API_VERSION.parse()?);
        Ok(headers)
    }
}

#[async_trait::async_trait]
impl BlobStore for VercelBlobStore {
    async fn exists(&self, pathname: &str) -> Result<bool> {
        Ok(self.head(pathname).await?.is_some())
    }

    async fn put(
        &self,
        pathname: &str,
        content_type: &str,
        body: Bytes,
        cache_control_max_age: u32,
    ) -> Result<PutReceipt> {
        let bearer = self.bearer()?;
        let url = self.api_url("/", &format!("pathname={pathname}"));
        let mut headers = self.api_headers(&bearer)?;
        // Grounded put headers (SDK put-helpers.ts): access is always required; the
        // suffix is pinned OFF because a content-addressed pathname must land verbatim.
        headers.insert("x-vercel-blob-access", "private".parse()?);
        headers.insert("x-content-type", content_type.parse()?);
        headers.insert("x-add-random-suffix", "0".parse()?);
        // The window the caller asked for rides the wire verbatim — a seam parameter
        // silently replaced by a local default is a contract lie.
        headers.insert(
            "x-cache-control-max-age",
            cache_control_max_age.to_string().parse()?,
        );

        let resp = self
            .http
            .put(&url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .context("blob provider put: request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("blob provider put: {status}: {}", truncate(&body));
        }
        Ok(PutReceipt {
            pathname: pathname.to_string(),
        })
    }

    async fn get(
        &self,
        pathname: &str,
        consistent: bool,
    ) -> Result<temper_substrate::blob_store::ByteStream> {
        let bearer = self.bearer()?;
        let mut url = self.read_url(pathname);
        if consistent {
            // Grounded (`useCache: false`): `?cache=0` asks the store host to bypass its
            // CDN cache — the consistent-read escape hatch.
            url.push_str("?cache=0");
        }
        let resp = self
            .http
            .get(&url)
            .header("authorization", format!("Bearer {bearer}"))
            .send()
            .await
            .with_context(|| format!("blob provider get {pathname}: request failed"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "blob provider get {pathname}: {status}: {}",
                truncate(&body)
            );
        }
        use futures_util::StreamExt;
        let label = pathname.to_string();
        let stream = resp
            .bytes_stream()
            .map(move |chunk| chunk.map_err(|e| anyhow::anyhow!("blob provider get {label}: {e}")));
        Ok(Box::pin(stream))
    }

    async fn head(&self, pathname: &str) -> Result<Option<BlobHead>> {
        let bearer = self.bearer()?;
        let object_url = self.read_url(pathname);
        let url = self.api_url("/", &format!("url={}", urlencode(&object_url)));
        let resp = self
            .http
            .get(&url)
            .headers(self.api_headers(&bearer)?)
            .send()
            .await
            .context("blob provider head: request failed")?;
        match resp.status() {
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            status if status.is_success() => {
                // Grounded head response shape (SDK HeadBlobApiResponse): `contentType` and
                // `size` are the two fields the read path needs.
                #[derive(serde::Deserialize)]
                struct HeadResponse {
                    #[serde(default, rename = "contentType")]
                    content_type: Option<String>,
                    #[serde(default)]
                    size: Option<i64>,
                }
                let parsed: HeadResponse = resp
                    .json()
                    .await
                    .context("blob provider head: unreadable response")?;
                // A provider 200 without `size` is contract drift, not a zero-byte
                // blob: a success-shaped `BlobHead { content_bytes: 0 }` would feed the
                // D4 commit gate a lie. Fail loudly instead.
                let size = parsed
                    .size
                    .context("blob provider head: response missing size")?;
                Ok(Some(BlobHead {
                    content_type: parsed.content_type,
                    content_bytes: size,
                }))
            }
            status => {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!(
                    "blob provider head {pathname}: {status}: {}",
                    truncate(&body)
                );
            }
        }
    }
}

fn truncate(s: &str) -> &str {
    match s.char_indices().nth(200) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn store_with(token: &'static str) -> VercelBlobStore {
        VercelBlobStore::new(BlobConfig {
            store_id: "abc123".into(),
            read_write_token: Some(token.into()),
            credential_mode: crate::config::BlobCredentialMode::Token,
            oidc_token_source: std::sync::Arc::new(|| None),
            max_bytes: 1,
            allowlist: vec!["image/png".into()],
            single_request_max_bytes: 1,
        })
    }

    // The read path (D6) streams unbuffered: byte count out of the stream must equal
    // byte count the provider holds, in order, with the bearer on the request.
    #[tokio::test]
    async fn get_streams_the_object_bytes_through_the_private_host() {
        let server = MockServer::start().await;
        let store = store_with("tok-1").with_test_endpoints(&server.uri());
        Mock::given(method("GET"))
            .and(path("/ab/abcd"))
            .and(header("authorization", "Bearer tok-1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(Bytes::from_static(b"blob-bytes-here"))
                    .insert_header("content-type", "image/png"),
            )
            .mount(&server)
            .await;

        let mut stream = store.get("ab/abcd", false).await.unwrap();
        use futures_util::StreamExt;
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, b"blob-bytes-here");
    }

    // FAILS IF: the consistent-read escape hatch (`?cache=0`) ever disappears — the
    // erasure task's post-delete verification needs it.
    #[tokio::test]
    async fn consistent_get_asks_the_provider_to_bypass_its_cache() {
        let server = MockServer::start().await;
        let store = store_with("tok-1").with_test_endpoints(&server.uri());
        Mock::given(method("GET"))
            .and(path("/ab/abcd"))
            .and(query_param("cache", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(Bytes::from_static(b"x")))
            .mount(&server)
            .await;

        let mut stream = store.get("ab/abcd", true).await.unwrap();
        use futures_util::StreamExt;
        assert!(stream.next().await.is_some());
    }

    // The put wire shape, grounded against the SDK: PUT with the pathname as the
    // query, access pinned private, the suffix pinned off, and the store id on the
    // header the OIDC path requires the API to see. The cache window the CALLER
    // asked for rides the wire — the commit path asks `IMMUTABLE_CACHE_MAX_AGE` and
    // the provider must apply that, never a local default.
    #[tokio::test]
    async fn put_sends_the_grounded_wire_shape() {
        let server = MockServer::start().await;
        let store = store_with("tok-1").with_test_endpoints(&server.uri());
        let asked = crate::services::blob_service::IMMUTABLE_CACHE_MAX_AGE;
        Mock::given(method("PUT"))
            .and(path("/api/blob/"))
            .and(query_param("pathname", "ab/abcd"))
            .and(header("authorization", "Bearer tok-1"))
            .and(header("x-vercel-blob-store-id", "abc123"))
            .and(header("x-api-version", "12"))
            .and(header("x-vercel-blob-access", "private"))
            .and(header("x-content-type", "image/png"))
            .and(header("x-add-random-suffix", "0"))
            // FAILS IF: the asked-for window is silently discarded for a local
            // default — the seam parameter is a contract, not a suggestion.
            .and(header("x-cache-control-max-age", asked.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://abc123.private.blob.vercel-storage.com/ab/abcd",
                "downloadUrl": "https://abc123.private.blob.vercel-storage.com/ab/abcd",
                "pathname": "ab/abcd",
                "contentType": "image/png",
                "etag": "\"etag\"",
            })))
            .mount(&server)
            .await;

        let receipt = store
            .put("ab/abcd", "image/png", Bytes::from_static(b"body"), asked)
            .await
            .unwrap();
        assert_eq!(receipt.pathname, "ab/abcd");
    }

    // FAILS IF: head stops distinguishing "present" from "absent" — the commit gate
    // (D4) refuses on this answer, and a wrong None/Some flips a valid commit or
    // admits a smuggled one.
    #[tokio::test]
    async fn head_resolves_present_and_absent() {
        let server = MockServer::start().await;
        let store = store_with("tok-1").with_test_endpoints(&server.uri());

        let _url_path = "/api/blob/";
        Mock::given(method("GET"))
            .and(path("/api/blob/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pathname": "ab/abcd",
                "contentType": "image/png",
                "size": 14,
            })))
            .mount(&server)
            .await;

        let head = store.head("ab/abcd").await.unwrap().unwrap();
        assert_eq!(head.content_type.as_deref(), Some("image/png"));
        assert_eq!(head.content_bytes, 14);
        assert!(store.exists("ab/abcd").await.unwrap());

        // The head targeted the content-addressed object: the `url` param carries it.
        let reqs = server.received_requests().await.unwrap();
        let query = reqs[0].url.query().unwrap().to_string();
        assert!(query.contains("url="), "{query}");
        assert!(query.contains("ab/abcd"), "{query}");
    }

    #[tokio::test]
    async fn head_answers_none_when_the_provider_holds_nothing_there() {
        let server = MockServer::start().await;
        let store = store_with("tok-1").with_test_endpoints(&server.uri());
        Mock::given(method("GET"))
            .and(path("/api/blob/"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let head = store.head("zz/zzzz").await.unwrap();
        assert!(head.is_none());
    }

    // FAILS IF: a provider failure is swallowed into a success-shaped result — the
    // commit gate must see the failure, not a made-up absence.
    #[tokio::test]
    async fn provider_failure_surfaces_the_status() {
        let server = MockServer::start().await;
        let store = store_with("tok-1").with_test_endpoints(&server.uri());
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let err = store
            .put("ab/abcd", "image/png", Bytes::from_static(b"b"), 100)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("503"), "{err}");
    }

    // FAILS IF: a 200 that omits `size` is swallowed into a success-shaped
    // `content_bytes: 0` — provider contract drift must reach the commit gate as a
    // failure, never as a made-up zero (B-S2, final-pass review).
    #[tokio::test]
    async fn head_refuses_a_success_response_without_a_size() {
        let server = MockServer::start().await;
        let store = store_with("tok-1").with_test_endpoints(&server.uri());
        Mock::given(method("GET"))
            .and(path("/api/blob/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pathname": "ab/abcd",
                "contentType": "image/png",
            })))
            .mount(&server)
            .await;

        let err = store.head("ab/abcd").await.unwrap_err().to_string();
        assert!(err.contains("missing size"), "{err}");
    }

    // FAILS IF: the provider client loses its read bound — a stalled provider used to
    // hang every blob door indefinitely (B-S1, final-pass review). The mock holds the
    // response far longer than the witness client's read window; the put must give up
    // inside it instead of waiting out the stall.
    #[tokio::test]
    async fn stalled_provider_put_fails_within_the_read_window() {
        let server = MockServer::start().await;
        let store = store_with("tok-1")
            .with_test_endpoints(&server.uri())
            .with_test_timeouts(Duration::from_millis(100), Duration::from_millis(300));
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .mount(&server)
            .await;

        let started = std::time::Instant::now();
        let result = store
            .put("ab/abcd", "image/png", Bytes::from_static(b"body"), 100)
            .await;
        let elapsed = started.elapsed();
        assert!(
            result.is_err(),
            "the stalled put must fail, not wait out the mock"
        );
        // Bounded: the read window tripped well inside the mock's 30s hold.
        assert!(elapsed < Duration::from_secs(5), "put took {elapsed:?}");
    }
}
