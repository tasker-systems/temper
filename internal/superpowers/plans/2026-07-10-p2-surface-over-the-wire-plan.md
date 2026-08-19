# P2 — Carry `Surface` Over the Wire via `X-Temper-Surface`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the calling surface survive the HTTP boundary, so a cloud-mode CLI write is attributed to `<handle>@cli` instead of `<handle>@web`, and a future `temper-rb` write is attributed to `<handle>@sdk`.

**Architecture:** `temper-client`'s `HttpClient` gains a `Surface` field set at construction and sends it as the `X-Temper-Surface` header on every request, exactly as it already does for `X-Temper-Device-Id`. `temper-api` replaces its 21 hardcoded `Surface::ApiHttp` literals with a `RequestSurface` extractor that parses the header through a `&str -> Option<Surface>` allowlist of exactly `{cli, sdk}`, degrading everything else — absent, unparseable, and `mcp` — to `Surface::ApiHttp`. The header is documented once in the OpenAPI contract by a path-level parameter injected after the router merge.

**Tech Stack:** Rust, axum 0.8, utoipa 5.4 + utoipa-axum 0.2, reqwest, sqlx, cargo-nextest, wiremock.

## Global Constraints

- **Surface is provenance, never authorization.** A bad header value degrades to `Surface::ApiHttp`; it never rejects the request, never 500s, never changes what the caller may do.
- **The allowlist is exactly `{cli, sdk}`.** `mcp` is excluded deliberately: `temper-mcp` reaches `DbBackend` in-process, so a remote caller claiming `mcp` is lying by construction.
- **Parse, don't validate.** The allowlist is expressed once as a `&str -> Option<Surface>` function. No handler inspects a raw string.
- **No new migrations in this plan.** A sibling session owns task `019f4965` (unique constraint on `kb_entities(profile_id, name)`) and is authoring migrations right now. If this work somehow needs one, stop and coordinate rather than picking a number.
- **Do not touch `provision_profile_entities` or `crates/temper-services/src/services/profile_service.rs`** — same sibling-session reason.
- `#[expect(lint, reason = "...")]` over `#[allow]`. All public types derive `Debug`.
- Every crate builds and lints clean under `--all-features`; clippy runs with `-D warnings`.
- Run `cargo fmt --all` before every commit. Subagents habitually forget this.

## Background: what P1 already landed

Do **not** redo any of this:

- `Surface::Sdk` exists in `crates/temper-workflow/src/operations/surface.rs` and serializes snake_case to `"sdk"`.
- `Surface::marker()` is an inherent method with an exhaustive match: `CliCloud => "cli"`, `Mcp => "mcp"`, `ApiHttp => "web"`, `Sdk => "sdk"`.
- `Surface::ALL` drives emitter provisioning; migration `20260709000030_backfill_sdk_emitter_entities.sql` backfilled `<handle>@sdk` and is **applied to production**.
- `crates/temper-substrate/src/writes.rs:52` `resolve_emitter(pool, profile, surface: &str)` resolves the natural key `<handle>@<marker>` with `fetch_one`.
- `tests/e2e/tests/sdk_emitter_entity_e2e.rs` exists and its module doc explicitly defers the wire-level assertion to this task.

## File Structure

| File | Responsibility |
|---|---|
| `crates/temper-workflow/src/operations/surface.rs` (modify) | Owns the `SURFACE_HEADER` name constant, so client and server cannot spell the header differently. |
| `crates/temper-client/src/http.rs` (modify) | `HttpClient` holds a `Surface`; `apply_surface_header` sends it. |
| `crates/temper-client/src/lib.rs` (modify) | `TemperClient::new` / `with_token` take a `Surface`. |
| `crates/temper-client/src/config.rs` (modify) | `build_client_from` / `build_client` take a `Surface`. |
| `crates/temper-cli/src/actions/runtime.rs` (modify) | The CLI's single client-construction site declares `Surface::CliCloud`. |
| `crates/temper-cli/src/cloud_backend/backend.rs` (modify) | Correct the stale comment that concedes the bug this task fixes. |
| `crates/temper-api/src/middleware/surface.rs` (create) | `parse_trusted` allowlist + `RequestSurface` extractor. The one place a raw header string is read. |
| `crates/temper-api/src/middleware/mod.rs` (modify) | Register the module. |
| `crates/temper-api/src/handlers/*.rs` (modify, 9 files) | Replace `Surface::ApiHttp` with the extracted surface. |
| `crates/temper-api/src/openapi.rs` (modify) | `SurfaceHeaderAddon` — a `Modify` impl building the path-level header parameter. |
| `crates/temper-api/src/routes.rs` (modify) | Apply the addon inside `openapi_spec()`, after the router merge. |
| `openapi.json` (regenerate) | The committed contract artifact. |
| `tests/e2e/tests/surface_attribution_e2e.rs` (create) | Wire-level proof: `@cli`, `@sdk`, `@web` land where they should. |

---

### Task 1: The header name constant

`SURFACE_HEADER` must live somewhere both `temper-client` and `temper-api` can see. `temper-client` already depends on `temper-workflow` (verified in `crates/temper-client/Cargo.toml`), and `Surface` already lives there, so `surface.rs` is the home.

**Files:**
- Modify: `crates/temper-workflow/src/operations/surface.rs`

**Interfaces:**
- Produces: `temper_workflow::operations::SURFACE_HEADER: &'static str` (re-exported from `surface`, matching how `Surface` is re-exported).

- [ ] **Step 1: Write the failing test**

Append to the existing `mod tests` block in `crates/temper-workflow/src/operations/surface.rs`:

```rust
    /// Both ends of the wire spell the header from this constant. A literal on either side
    /// would be a silent, untestable drift.
    #[test]
    fn surface_header_name_is_stable() {
        assert_eq!(SURFACE_HEADER, "X-Temper-Surface");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-workflow surface_header_name_is_stable`
Expected: FAIL — `cannot find value 'SURFACE_HEADER' in this scope`.

- [ ] **Step 3: Add the constant**

Insert immediately above `/// The originating surface of a command.` in `crates/temper-workflow/src/operations/surface.rs`:

```rust
/// The HTTP header a remote client uses to claim its calling surface.
///
/// The value is the claimed surface's [`Surface::marker`] spelling — the same `<marker>` half
/// of the `<handle>@<marker>` emitter natural key the write will be attributed to. The header
/// names the emitter the caller claims to be.
///
/// The server trusts exactly `cli` and `sdk`; everything else degrades to [`Surface::ApiHttp`].
/// Surface is provenance, never authorization.
pub const SURFACE_HEADER: &str = "X-Temper-Surface";
```

- [ ] **Step 4: Re-export it**

In `crates/temper-workflow/src/operations/mod.rs`, find the line re-exporting `Surface` from the `surface` module (it will read `pub use surface::Surface;` or list `Surface` among several names) and extend it to also export `SURFACE_HEADER`. For example, `pub use surface::Surface;` becomes:

```rust
pub use surface::{Surface, SURFACE_HEADER};
```

Do not guess the surrounding line — open the file and edit the actual `pub use surface::...` statement.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p temper-workflow surface`
Expected: PASS — `surface_header_name_is_stable`, plus the four pre-existing surface tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/temper-workflow/src/operations/surface.rs crates/temper-workflow/src/operations/mod.rs
git commit -m "P2: SURFACE_HEADER constant shared by client and server"
```

---

### Task 2: `HttpClient` carries and sends the surface

The surface is construction state, not a per-call argument: a client *is* a surface for its whole life. Every construction site must name one — no defaulted setter, because a default would silently recreate the `@web` bug this task exists to fix.

**Files:**
- Modify: `crates/temper-client/src/http.rs`
- Modify: `crates/temper-client/src/lib.rs`
- Modify: `crates/temper-client/src/config.rs`
- Modify: `crates/temper-client/tests/retry_tests.rs`
- Modify: `crates/temper-client/tests/segments_client_test.rs`

**Interfaces:**
- Consumes: `temper_workflow::operations::{Surface, SURFACE_HEADER}` (Task 1).
- Produces:
  - `HttpClient::new(base_url: &str, device_id: Option<String>, surface: Surface, token_store: Option<Arc<dyn TokenStore>>) -> Self`
  - `HttpClient::with_token_override(base_url: &str, device_id: Option<String>, surface: Surface, token: String) -> Self`
  - `TemperClient::new(base_url: &str, device_id: Option<String>, surface: Surface, store: Arc<dyn TokenStore>) -> Self`
  - `TemperClient::with_token(base_url: &str, device_id: Option<String>, surface: Surface, token: String, store: Arc<dyn TokenStore>) -> Self`
  - `config::build_client_from(config: &TemperConfig, store: Arc<dyn TokenStore>, surface: Surface) -> Result<TemperClient>`
  - `config::build_client(store: Arc<dyn TokenStore>, surface: Surface) -> Result<TemperClient>`

  In every signature `surface` sits immediately after `device_id` (they are the same kind of thing: client identity carried as a header). In `build_client_from` / `build_client` it goes last, so the existing leading arguments are untouched at call sites.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-client/tests/retry_tests.rs`. Read the top of that file first and reuse its existing imports and `MockServer` setup idiom rather than duplicating it.

```rust
#[tokio::test]
async fn sends_surface_header_on_every_request() {
    use temper_workflow::operations::Surface;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .and(header("X-Temper-Surface", "cli"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpClient::new(&server.uri(), None, Surface::CliCloud, None);
    let req = client.get("/api/health");
    let _ = client.send(req, reqwest::Method::GET, "/api/health", None).await;
    // `expect(1)` on the mock asserts the header matched; drop verifies it.
}

#[tokio::test]
async fn sdk_client_sends_sdk_marker() {
    use temper_workflow::operations::Surface;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .and(header("X-Temper-Surface", "sdk"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpClient::new(&server.uri(), None, Surface::Sdk, None);
    let req = client.get("/api/health");
    let _ = client.send(req, reqwest::Method::GET, "/api/health", None).await;
}
```

`HttpClient::send`'s exact signature is at `crates/temper-client/src/http.rs` around line 195 (`pub async fn send(&self, ...)`) — read it and match the argument list. If `send` takes an `Option<&str>` token as its last argument, pass `None`; if it takes something else, adapt. Add `wiremock::matchers::header` to the file's `use` list.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-client --test retry_tests sends_surface_header`
Expected: FAIL — `this function takes 3 arguments but 4 arguments were supplied`.

- [ ] **Step 3: Add the field and the header**

In `crates/temper-client/src/http.rs`:

Add the import alongside the existing ones:

```rust
use temper_workflow::operations::{Surface, SURFACE_HEADER};
```

Add the field to the struct (after `device_id`):

```rust
    surface: Surface,
```

Add it to the `Debug` impl, after the `device_id` field line:

```rust
            .field("surface", &self.surface)
```

Change the `new` constructor:

```rust
    /// Construct an `HttpClient` with a [`TokenStore`] for bearer-token
    /// resolution. Cloud sessions pass an `Arc<MemoryTokenStore>`; local
    /// sessions pass an `Arc<DiskTokenStore>`. Tests that don't care about
    /// auth can pass `None` and use [`HttpClient::with_token_override`] instead.
    ///
    /// `surface` is construction state, not a per-call argument — a client *is* a
    /// surface for its whole life. There is deliberately no default: a defaulted
    /// surface would silently attribute every write to `@web`.
    pub fn new(
        base_url: &str,
        device_id: Option<String>,
        surface: Surface,
        token_store: Option<Arc<dyn TokenStore>>,
    ) -> Self {
        let inner = Client::builder()
            .timeout(Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECS))
            .build()
            .expect("failed to build reqwest client");

        Self {
            inner,
            base_url: base_url.trim_end_matches('/').to_owned(),
            device_id,
            surface,
            token_override: None,
            token_store,
        }
    }
```

Change `with_token_override`:

```rust
    pub fn with_token_override(
        base_url: &str,
        device_id: Option<String>,
        surface: Surface,
        token: String,
    ) -> Self {
        Self {
            token_override: Some(token),
            ..Self::new(base_url, device_id, surface, None)
        }
    }
```

Rename `apply_device_header` to `apply_identity_headers` and add the surface header. Both headers describe *who is calling*, and both ride every request including GETs:

```rust
    /// Attach the client-identity headers: device id (when set) and the calling surface
    /// (always). Both ride every request, including GETs — they describe the caller, not
    /// the operation.
    fn apply_identity_headers(&self, req: RequestBuilder) -> RequestBuilder {
        let req = req.header(SURFACE_HEADER, self.surface.marker());
        if let Some(id) = &self.device_id {
            req.header("X-Temper-Device-Id", id.as_str())
        } else {
            req
        }
    }
```

Update all five verb methods (`get`, `post`, `patch`, `delete`, `put`) to call `self.apply_identity_headers(...)` instead of `self.apply_device_header(...)`.

Also update the struct's doc comment — it currently says "inject the `X-Temper-Device-Id` header when a device ID has been set." Replace that sentence with:

```rust
/// All request methods prepend `base_url` to the given path, inject the
/// `X-Temper-Surface` header naming this client's surface, and inject the
/// `X-Temper-Device-Id` header when a device ID has been set.
```

- [ ] **Step 4: Fix the in-file unit tests**

`crates/temper-client/src/http.rs` has four `HttpClient::new(...)` / `with_token_override(...)` calls inside its own `#[cfg(test)] mod tests` (near lines 515, 524, 554, 570, 650). Add `Surface::CliCloud` as the third argument to each. Add `use temper_workflow::operations::Surface;` to the test module if it is not already in scope via `use super::*`.

- [ ] **Step 5: Run the client's own tests**

Run: `cargo test -p temper-client --lib`
Expected: PASS.

Run: `cargo test -p temper-client --test retry_tests`
Expected: FAIL still — `retry_tests.rs`'s three pre-existing `HttpClient::new(&server.uri(), None, None)` calls (lines 34, 57, 79) need the new argument. Add `Surface::CliCloud` to each, with `use temper_workflow::operations::Surface;` at the top. Re-run.
Expected: PASS, including the two new header tests.

- [ ] **Step 6: Thread the surface through `TemperClient`**

In `crates/temper-client/src/lib.rs`:

```rust
    /// Create a new client targeting `base_url`.
    ///
    /// `device_id` is sent as `X-Temper-Device-Id` on every request for
    /// per-device manifest tracking. `surface` is sent as `X-Temper-Surface`
    /// and determines which `<handle>@<marker>` emitter the server attributes
    /// this client's writes to. `store` is the source of truth for token resolution.
    pub fn new(
        base_url: &str,
        device_id: Option<String>,
        surface: temper_workflow::operations::Surface,
        store: Arc<dyn auth::TokenStore>,
    ) -> Self {
        Self {
            http: http::HttpClient::new(base_url, device_id, surface, Some(store.clone())),
            oauth_config: None,
            store,
        }
    }

    /// Create a new client with a pre-resolved token override.
    ///
    /// Used by `build_client_from` after resolving the current token from
    /// the store — the override path keeps the request path off any further
    /// store reads for the lifetime of this client. The store is still held
    /// for refresh / logout / status operations.
    pub fn with_token(
        base_url: &str,
        device_id: Option<String>,
        surface: temper_workflow::operations::Surface,
        token: String,
        store: Arc<dyn auth::TokenStore>,
    ) -> Self {
        Self {
            http: http::HttpClient::with_token_override(base_url, device_id, surface, token),
            oauth_config: None,
            store,
        }
    }
```

- [ ] **Step 7: Thread it through the config builders**

In `crates/temper-client/src/config.rs`:

```rust
pub fn build_client_from(
    config: &TemperConfig,
    store: std::sync::Arc<dyn crate::auth::TokenStore>,
    surface: temper_workflow::operations::Surface,
) -> crate::error::Result<crate::TemperClient> {
    let url = api_url(config);
    let auth = store.load()?;
    let device_id = auth.as_ref().and_then(|a| a.device_id.clone());

    let client = if let Some(auth) = auth {
        crate::TemperClient::with_token(
            &url,
            device_id,
            surface,
            secrecy::ExposeSecret::expose_secret(&auth.access_token).to_string(),
            store,
        )
    } else {
        crate::TemperClient::new(&url, device_id, surface, store)
    };

    let client = match oauth_config(config) {
        Ok(oauth) => client.with_oauth(oauth),
        Err(e) => {
            tracing::debug!("OAuth config not available: {e}");
            client
        }
    };

    Ok(client)
}
```

And:

```rust
pub fn build_client(
    store: std::sync::Arc<dyn crate::auth::TokenStore>,
    surface: temper_workflow::operations::Surface,
) -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    build_client_from(&config, store, surface)
}
```

- [ ] **Step 8: Fix the remaining call sites in `temper-client`**

- `crates/temper-client/src/config.rs` `mod tests`: `build_client(store)` near line 406 and `build_client_from(&config, store)` near line 431 — add `Surface::CliCloud`.
- `crates/temper-client/tests/integration_test.rs` line 34: `build_client(store)` — add `Surface::CliCloud`, import `temper_workflow::operations::Surface`.
- `crates/temper-client/tests/segments_client_test.rs` line 22: `TemperClient::with_token(...)` — insert `Surface::CliCloud` as the third argument, import `Surface`.

- [ ] **Step 9: Run the whole client crate**

Run: `cargo test -p temper-client --all-features`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
cargo fmt --all
git add crates/temper-client
git commit -m "P2: HttpClient carries a Surface and sends X-Temper-Surface"
```

---

### Task 3: The CLI declares its surface, and the stale comment goes

`temper-cli` builds its client in exactly one place: `build_config_store_and_client()` in `crates/temper-cli/src/actions/runtime.rs:122`. Both `with_client` and `assemble_cloud_backend` route through it, so one edit covers every CLI code path.

**Files:**
- Modify: `crates/temper-cli/src/actions/runtime.rs`
- Modify: `crates/temper-cli/src/cloud_backend/backend.rs`

**Interfaces:**
- Consumes: `config::build_client_from(config, store, surface)` (Task 2).

- [ ] **Step 1: Declare the surface at the construction site**

In `crates/temper-cli/src/actions/runtime.rs`, change the `build_client_from` call inside `build_config_store_and_client`:

```rust
    let client = build_client_from(
        &config,
        store.clone(),
        temper_workflow::operations::Surface::CliCloud,
    )
    .map_err(|e| TemperError::Api(e.to_string()))?;
```

- [ ] **Step 2: Build to verify**

Run: `cargo build -p temper-cli`
Expected: PASS.

- [ ] **Step 3: Correct the stale comment**

`crates/temper-cli/src/cloud_backend/backend.rs` around line 327 carries a comment conceding exactly the bug this task fixes:

> `origin` is unused here: this backend forwards over HTTP, and the server attributes the event to the surface it actually received (`Surface::ApiHttp`). Carrying the CLI's origin across the wire would need a header or payload field — a separate concern from making the parameter honest for in-process callers (MCP, API).

Replace it with:

```rust
        // `origin` is unused *here* because the surface is carried by the client, not the
        // command: `HttpClient` was constructed with `Surface::CliCloud` and sends
        // `X-Temper-Surface: cli` on every request. The server parses that header and
        // attributes the event to `<handle>@cli`. The parameter stays for in-process
        // callers (MCP, API), whose backend reads `cmd.origin` directly.
```

Search the rest of `backend.rs` for any other comment asserting that the origin is dropped at the HTTP boundary (`rg -n "attributes the event to the surface it actually received" crates/temper-cli/`) and correct each.

- [ ] **Step 4: Verify the CLI crate**

Run: `cargo clippy -p temper-cli --all-features -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/temper-cli
git commit -m "P2: the CLI declares Surface::CliCloud at client construction"
```

---

### Task 4: The server-side allowlist and extractor

This is where a raw header string becomes a `Surface`, and the only place in the server that ever sees one. The allowlist *is* the parser: a `&str -> Option<Surface>` returning `None` for everything untrusted.

`RequestSurface` is an infallible `FromRequestParts` extractor reading headers directly, not a middleware extension. `require_auth` only wraps authenticated routes; an extractor works on every route and requires no middleware edit.

**Files:**
- Create: `crates/temper-api/src/middleware/surface.rs`
- Modify: `crates/temper-api/src/middleware/mod.rs`

**Interfaces:**
- Consumes: `temper_workflow::operations::{Surface, SURFACE_HEADER}` (Task 1).
- Produces: `crate::middleware::surface::RequestSurface(pub Surface)`, an axum extractor with `type Rejection = std::convert::Infallible`.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/src/middleware/surface.rs` containing *only* the test module for now, so the test genuinely fails to compile against a missing function:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use temper_workflow::operations::Surface;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(SURFACE_HEADER, value.parse().expect("valid header value"));
        h
    }

    /// The client's send-side spelling and the server's accept-side allowlist are the same
    /// strings. Deriving the test input from `marker()` means they cannot drift.
    #[test]
    fn trusted_markers_round_trip_from_the_client_spelling() {
        assert_eq!(parse_trusted(Surface::CliCloud.marker()), Some(Surface::CliCloud));
        assert_eq!(parse_trusted(Surface::Sdk.marker()), Some(Surface::Sdk));
    }

    /// `temper-mcp` reaches `DbBackend` in-process, so a remote caller claiming `mcp` is
    /// lying by construction. It is untrusted, not merely unrecognized.
    #[test]
    fn mcp_is_not_trusted() {
        assert_eq!(parse_trusted(Surface::Mcp.marker()), None);
    }

    /// `web` is what everything degrades *to*. A caller cannot claim it either — claiming it
    /// and being degraded to it are the same outcome, so the allowlist stays exactly two.
    #[test]
    fn web_is_not_claimable() {
        assert_eq!(parse_trusted(Surface::ApiHttp.marker()), None);
    }

    #[test]
    fn garbage_and_empty_are_not_trusted() {
        assert_eq!(parse_trusted(""), None);
        assert_eq!(parse_trusted("   "), None);
        assert_eq!(parse_trusted("CLI"), None);
        assert_eq!(parse_trusted("cli; drop table"), None);
        assert_eq!(parse_trusted("sdkx"), None);
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert_eq!(parse_trusted("  cli  "), Some(Surface::CliCloud));
    }

    // --- resolve_surface: the degrade direction, which must never reject ---

    #[test]
    fn absent_header_degrades_to_web() {
        assert_eq!(resolve_surface(&HeaderMap::new()), Surface::ApiHttp);
    }

    #[test]
    fn untrusted_header_degrades_to_web() {
        assert_eq!(resolve_surface(&headers_with("mcp")), Surface::ApiHttp);
        assert_eq!(resolve_surface(&headers_with("nonsense")), Surface::ApiHttp);
        assert_eq!(resolve_surface(&headers_with("")), Surface::ApiHttp);
    }

    #[test]
    fn trusted_header_resolves() {
        assert_eq!(resolve_surface(&headers_with("cli")), Surface::CliCloud);
        assert_eq!(resolve_surface(&headers_with("sdk")), Surface::Sdk);
    }

    /// A header whose bytes are not valid ASCII cannot even be `to_str`'d. It degrades; it
    /// must not panic and must not 500.
    #[test]
    fn non_ascii_header_degrades_to_web() {
        let mut h = HeaderMap::new();
        h.insert(
            SURFACE_HEADER,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).expect("opaque bytes"),
        );
        assert_eq!(resolve_surface(&h), Surface::ApiHttp);
    }
}
```

Register the module — in `crates/temper-api/src/middleware/mod.rs`:

```rust
pub mod auth;
pub mod internal_auth;
pub mod surface;
pub mod system_access;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-api --lib middleware::surface`
Expected: FAIL — `cannot find function 'parse_trusted' in this scope`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/temper-api/src/middleware/surface.rs`, above the test module:

```rust
//! Resolving the caller's claimed surface from the `X-Temper-Surface` request header.
//!
//! Surface is **provenance, never authorization**. It selects which `<handle>@<marker>` emitter
//! entity a write is attributed to in the event ledger. It grants nothing. A bad value therefore
//! degrades — it never rejects, and it never 500s.

use axum::extract::FromRequestParts;
use axum::http::{request::Parts, HeaderMap};
use std::convert::Infallible;
use std::future::Future;

use temper_workflow::operations::{Surface, SURFACE_HEADER};

/// Parse a client-claimed surface marker into the surface it names.
///
/// This function **is** the allowlist. It trusts exactly two markers:
///
/// - `cli` — `temper-cli` in cloud mode, forwarding over HTTP.
/// - `sdk` — a generated SDK client (`temper-rb` and its successors).
///
/// Everything else is `None`, including `mcp`: `temper-mcp` reaches `DbBackend` in-process and
/// never crosses this boundary, so a remote caller claiming `mcp` is lying by construction. And
/// including `web`, which is what an unclaimed request degrades to anyway.
fn parse_trusted(raw: &str) -> Option<Surface> {
    match raw.trim() {
        "cli" => Some(Surface::CliCloud),
        "sdk" => Some(Surface::Sdk),
        _ => None,
    }
}

/// Resolve the surface of an inbound request, degrading to [`Surface::ApiHttp`] (`web`) whenever
/// the header is absent, unreadable, or not on the allowlist.
///
/// Never fails. An untrusted claim is logged at debug — it is ordinary traffic (every browser
/// request omits the header), not an anomaly worth a warning.
fn resolve_surface(headers: &HeaderMap) -> Surface {
    let Some(raw) = headers.get(SURFACE_HEADER) else {
        return Surface::ApiHttp;
    };
    let Ok(value) = raw.to_str() else {
        tracing::debug!("{SURFACE_HEADER} is not valid ASCII; attributing to web");
        return Surface::ApiHttp;
    };
    match parse_trusted(value) {
        Some(surface) => surface,
        None => {
            tracing::debug!(claimed = %value, "untrusted {SURFACE_HEADER}; attributing to web");
            Surface::ApiHttp
        }
    }
}

/// The surface this request was received on, resolved from `X-Temper-Surface`.
///
/// Handlers take this extractor instead of hardcoding [`Surface::ApiHttp`], and pass the inner
/// value as their command's `origin`. Extraction is infallible by design: an unparseable claim
/// degrades to `web` rather than rejecting the request.
#[derive(Debug, Clone, Copy)]
pub struct RequestSurface(pub Surface);

impl<S> FromRequestParts<S> for RequestSurface
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(RequestSurface(resolve_surface(&parts.headers))))
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p temper-api --lib middleware::surface`
Expected: PASS — nine tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/temper-api/src/middleware/surface.rs crates/temper-api/src/middleware/mod.rs
git commit -m "P2: X-Temper-Surface allowlist parser and RequestSurface extractor"
```

---

### Task 5: Handlers use the extracted surface

Twenty-one sites across nine handler files hardcode `Surface::ApiHttp`. Each becomes the extracted value. Mechanical, but the count is the acceptance criterion — a missed site is a silently-still-broken endpoint.

**Files (exact sites, from `rg -n "Surface::ApiHttp" crates/temper-api/src/`):**
- Modify: `crates/temper-api/src/handlers/steward.rs:87,167`
- Modify: `crates/temper-api/src/handlers/resources.rs:104,216,312,348`
- Modify: `crates/temper-api/src/handlers/cognitive_maps.rs:79,113,212`
- Modify: `crates/temper-api/src/handlers/meta.rs:77`
- Modify: `crates/temper-api/src/handlers/segments.rs:47,74`
- Modify: `crates/temper-api/src/handlers/facets.rs:40`
- Modify: `crates/temper-api/src/handlers/edges.rs:73,112,150,188`
- Modify: `crates/temper-api/src/handlers/ingest.rs:145,225`
- Modify: `crates/temper-api/src/handlers/invocations.rs:58,93`

**Interfaces:**
- Consumes: `crate::middleware::surface::RequestSurface` (Task 4).

- [ ] **Step 1: Confirm the site count before touching anything**

Run: `rg -c "Surface::ApiHttp" crates/temper-api/src/ | sort`
Expected: nine files summing to 21. Record the number. If it is not 21, the tree has moved since this plan was written — stop and report rather than guessing.

- [ ] **Step 2: Convert one handler and verify the shape**

Start with `crates/temper-api/src/handlers/facets.rs` — a single site, no `Path`, so it isolates the extractor-ordering question.

Add the import:

```rust
use crate::middleware::surface::RequestSurface;
```

Change the handler signature and the command construction. **Extractor order matters in axum:** every `FromRequestParts` extractor must precede the single `FromRequest` body extractor (`Json<...>`), which must be last. Put `surface` immediately after `auth`:

```rust
pub async fn set_facet(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(payload): Json<FacetSetRequest>,
) -> ApiResult<Json<FacetAck>> {
```

and inside, replace `origin: Surface::ApiHttp,` with:

```rust
        origin: surface,
```

If `Surface` is now unused in that file's imports, drop it from the `use temper_workflow::operations::{...}` list. Clippy runs with `-D warnings`, so an unused import fails the build.

- [ ] **Step 3: Verify the one handler compiles and the crate's tests still pass**

Run: `cargo clippy -p temper-api --all-features -- -D warnings`
Expected: PASS.

- [ ] **Step 4: Convert the remaining eight files**

Apply the same three edits per file: add the `RequestSurface` import, add `RequestSurface(surface): RequestSurface` to each handler signature after `auth` (and before any `Json` body), replace `Surface::ApiHttp` with `surface`.

Two files need care:

**`segments.rs`** passes the surface positionally rather than as a struct field. Both handlers become:

```rust
pub async fn append_block_handler(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<AppendBlockPayload>,
) -> ApiResult<Json<BlocksResponse>> {
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .append_block(ResourceId::from(resource_id), payload, surface)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}
```

and correspondingly `finalize_ingest(ResourceId::from(resource_id), payload, surface)`.

**`resources.rs:104`** is inside `get`, a read (`ShowResource`), not a write. Convert it anyway — the command's `origin` should tell the truth regardless of whether the backend emits an event for it. Same for any other read commands encountered.

- [ ] **Step 5: Verify no site was missed**

Run: `rg -n "Surface::ApiHttp" crates/temper-api/src/`
Expected: **no matches.** The only remaining occurrences in the crate are in `crates/temper-api/tests/`, which construct commands directly and never cross the HTTP boundary — leave them.

- [ ] **Step 6: Run the API crate's test suite**

Run: `cargo clippy -p temper-api --all-features -- -D warnings`
Expected: PASS.

Run: `cargo nextest run -p temper-api --features test-db --test set_facet_test --test act_authorship_test --test nonauthored_act_correlation_test`
Expected: PASS. (Never run a bare `cargo nextest run -p temper-api` — it hangs enumerating the bin target. Always scope to `--test <name>`.)

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/temper-api/src/handlers
git commit -m "P2: handlers attribute writes to the received surface, not a hardcoded web"
```

---

### Task 6: Document the header in the OpenAPI contract

**The critical constraint, verified against `crates/temper-api/src/routes.rs:264`:** `openapi_spec()` seeds `OpenApiRouter::with_openapi(ApiDoc::openapi())` and *then* merges the routers. `ApiDoc::openapi()` runs its `modifiers(...)` at that seed point, when `paths` is still **empty** — P0 deleted the hand-maintained `paths(...)` list. `SecurityAddon` works only because it touches `components`, not `paths`. Adding `SurfaceHeaderAddon` to the `modifiers(...)` attribute would therefore be a silent no-op.

So the addon is written as a `Modify` impl (keeping the idiom and making it unit-testable) but **applied explicitly inside `openapi_spec()`, after `split_for_parts()`**.

The parameter attaches at **path-item** level, not per-operation. `utoipa::openapi::PathItem` has a `parameters: Option<Vec<Parameter>>` field, which OpenAPI defines as "parameters common to all operations in this path item." That is exactly true of a client-identity header, and it costs one entry per path rather than one per operation.

**Files:**
- Modify: `crates/temper-api/src/openapi.rs`
- Modify: `crates/temper-api/src/routes.rs`
- Regenerate: `openapi.json`

**Interfaces:**
- Consumes: `temper_workflow::operations::SURFACE_HEADER` (Task 1).
- Produces: `crate::openapi::SurfaceHeaderAddon`, a `utoipa::Modify` impl.

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `crates/temper-api/src/openapi.rs`:

```rust
    /// Every documented path carries the client-identity header, so a generated client can
    /// learn the header exists from the contract alone — which is the whole point of P0.
    ///
    /// Asserted against the *structure*, not the serialized string: a doc comment containing
    /// the header's name would make a `json.contains(..)` assertion pass vacuously.
    #[test]
    fn every_path_documents_the_surface_header() {
        use temper_workflow::operations::SURFACE_HEADER;

        let spec = crate::routes::openapi_spec();
        assert!(!spec.paths.paths.is_empty(), "spec has no paths to check");

        for (path, item) in spec.paths.paths.iter() {
            let params = item
                .parameters
                .as_ref()
                .unwrap_or_else(|| panic!("{path} has no path-level parameters"));
            assert!(
                params.iter().any(|p| p.name == SURFACE_HEADER),
                "{path} does not document {SURFACE_HEADER}",
            );
        }
    }

    /// The header is optional and never required: a browser omits it, and the server degrades.
    /// A `required: true` here would make every generated client demand it.
    #[test]
    fn the_surface_header_is_optional() {
        use temper_workflow::operations::SURFACE_HEADER;
        use utoipa::openapi::{path::ParameterIn, Required};

        let spec = crate::routes::openapi_spec();
        let (_, item) = spec.paths.paths.iter().next().expect("at least one path");
        let param = item
            .parameters
            .as_ref()
            .expect("path-level parameters")
            .iter()
            .find(|p| p.name == SURFACE_HEADER)
            .expect("surface header parameter");

        assert_eq!(param.required, Required::False);
        assert_eq!(param.parameter_in, ParameterIn::Header);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-api --lib openapi::tests::every_path_documents_the_surface_header`
Expected: FAIL — `/api/health has no path-level parameters`.

- [ ] **Step 3: Write the addon**

In `crates/temper-api/src/openapi.rs`, extend the imports:

```rust
use utoipa::openapi::path::{Parameter, ParameterBuilder, ParameterIn};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{ObjectBuilder, Required, Type};
use utoipa::{Modify, OpenApi};
```

Append below the existing `SecurityAddon` impl:

```rust
/// The `X-Temper-Surface` parameter, as documented on every path.
fn surface_header_parameter() -> Parameter {
    ParameterBuilder::new()
        .name(temper_workflow::operations::SURFACE_HEADER)
        .parameter_in(ParameterIn::Header)
        .required(Required::False)
        .description(Some(
            "The calling surface, for event-ledger attribution. Accepted values are `cli` \
             and `sdk`; an absent or unrecognized value attributes the write to `web`. This \
             is provenance, never authorization — an unrecognized value degrades, it never \
             rejects.",
        ))
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(["cli", "sdk"]))
                .build(),
        ))
        .build()
}

/// Documents `X-Temper-Surface` once, on every path item.
///
/// **Not registered in `ApiDoc`'s `modifiers(...)`.** `ApiDoc::openapi()` runs its modifiers at
/// the moment `routes::openapi_spec()` seeds the `OpenApiRouter` — before the routers merge, when
/// `paths` is still empty. `SecurityAddon` survives that only because it edits `components`.
/// This addon edits `paths`, so `openapi_spec()` applies it *after* `split_for_parts()`.
///
/// Attaching at path-item level (rather than per-operation) matches the OpenAPI semantics of
/// "parameters common to all operations in this path item," which is exactly what a
/// client-identity header is. It also means a newly registered route cannot forget the header.
pub(crate) struct SurfaceHeaderAddon;

impl Modify for SurfaceHeaderAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let name = temper_workflow::operations::SURFACE_HEADER;
        for item in openapi.paths.paths.values_mut() {
            let params = item.parameters.get_or_insert_with(Vec::new);
            // Idempotent: `modify` must not double-insert if ever applied twice.
            if !params.iter().any(|p| p.name == name) {
                params.push(surface_header_parameter());
            }
        }
    }
}
```

If `ObjectBuilder::enum_values` does not accept `Some([&str; 2])` on utoipa 5.4, drop the `.enum_values(...)` line entirely — the `Type::String` schema plus the description carries the contract. Do not fight the builder; the enum is a nicety, the parameter's presence is the requirement.

- [ ] **Step 4: Apply it after the merge**

In `crates/temper-api/src/routes.rs`:

```rust
pub fn openapi_spec() -> utoipa::openapi::OpenApi {
    use utoipa::Modify;

    let mut spec = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(public_routes())
        .merge(auth_only_routes())
        .merge(gated_routes())
        .split_for_parts()
        .1;

    // Applied here, not via `ApiDoc`'s `modifiers(...)`: those run against the seed spec, whose
    // `paths` map is empty until the merges above populate it.
    crate::openapi::SurfaceHeaderAddon.modify(&mut spec);
    spec
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p temper-api --lib openapi`
Expected: PASS — the two new tests plus the pre-existing `openapi_spec_is_valid`.

- [ ] **Step 6: Regenerate the committed contract**

The `openapi-check` gate diffs `openapi.json` against a fresh emission and fails CI otherwise. P0's session note predicted exactly this moment.

Run: `cargo make openapi`
Expected: `OpenAPI spec written: .../openapi.json (NNNNN bytes)`.

Run: `bash .github/scripts/check-openapi-spec.sh`
Expected: `openapi.json is up to date with the router`.

Run: `git diff --stat openapi.json`
Expected: `openapi.json` only, with additions on the order of one `parameters` block per path.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/temper-api/src/openapi.rs crates/temper-api/src/routes.rs openapi.json
git commit -m "P2: document X-Temper-Surface on every path in the OpenAPI contract"
```

---

### Task 7: End-to-end proof at the wire

`tests/e2e/tests/sdk_emitter_entity_e2e.rs`'s module doc says: *"The wire-level assertion (an `X-Temper-Surface: sdk` request attributing to `<handle>@sdk`) belongs to P2 — no client can send the marker yet."* This task discharges that debt, and proves the three acceptance criteria: a CLI write lands `@cli`, an `sdk` write lands `@sdk`, a browser write still lands `@web`.

`app.client` from the e2e harness is built through `build_client_from`, so after Task 2 it is a `Surface::CliCloud` client — it *is* the CLI path. The other surfaces are driven with `app.reqwest_client` and explicit headers, which is also the honest way to test the deny direction (no typed client can construct an `mcp` claim).

**Files:**
- Create: `tests/e2e/tests/surface_attribution_e2e.rs`
- Modify: `tests/e2e/tests/common/mod.rs`
- Modify: `tests/e2e/tests/sdk_emitter_entity_e2e.rs`

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Fix the harness for the new signature**

`tests/e2e/tests/common/mod.rs` calls `build_client_from(&temper_config, store)` at lines 337 and 430. Add the surface — the harness client stands in for the CLI:

```rust
    let client = temper_client::config::build_client_from(
        &temper_config,
        store,
        temper_workflow::operations::Surface::CliCloud,
    )
    .expect("Failed to build test client");
```

Confirm `temper-workflow` is a dev-dependency of the e2e crate:

Run: `rg -n "temper-workflow" tests/e2e/Cargo.toml`
Expected: a match (the suite already imports `temper_workflow::operations::Surface` in `sdk_emitter_entity_e2e.rs`). If absent, add `temper-workflow = { path = "../../crates/temper-workflow" }` under `[dev-dependencies]`.

- [ ] **Step 2: Write the failing test**

Create `tests/e2e/tests/surface_attribution_e2e.rs`:

```rust
#![cfg(feature = "test-db")]

//! Wire-level proof that `X-Temper-Surface` reaches the event ledger — the assertion
//! `sdk_emitter_entity_e2e.rs` deferred to P2.
//!
//! Before this task, `temper-cli`'s cloud backend constructed `Surface::CliCloud`, threaded it
//! through the command, and dropped it at the HTTP boundary: every cloud-mode CLI write was
//! attributed `<handle>@web`, and the `<handle>@cli` entity provisioned for every profile was
//! never resolved. These tests fail against that world.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_workflow::operations::{Surface, SURFACE_HEADER};

/// The emitter entity name on the most recent event for `handle`.
///
/// `kb_events.id` is UUIDv7, so `ORDER BY id DESC` is newest-first without needing to know
/// which anchor column a resource-create event populates.
async fn latest_emitter_for(pool: &PgPool, handle: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT e.name FROM kb_events ev \
         JOIN kb_entities e ON e.id = ev.emitter_entity_id \
         JOIN kb_profiles p ON p.id = e.profile_id \
         WHERE p.handle = $1 \
         ORDER BY ev.id DESC LIMIT 1",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("an event exists for this profile")
}

async fn handle_of_only_profile(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>("SELECT handle FROM kb_profiles ORDER BY id DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a profile exists")
}

/// POST a resource-creating request with an explicit (or absent) surface header, and return the
/// emitter name the write was attributed to.
async fn create_and_read_emitter(
    app: &common::E2eTestApp,
    surface_header: Option<&str>,
) -> String {
    let mut req = app
        .reqwest_client
        .post(format!("{}/api/ingest", app.base_url()))
        .bearer_auth(&app.token)
        .json(&json!({
            "title": "surface attribution probe",
            "doc_type_name": "research",
            "context_ref": "@me/temper",
            "content": "probe body",
            "origin_uri": "",
            "open_meta": {},
        }));

    if let Some(value) = surface_header {
        req = req.header(SURFACE_HEADER, value);
    }

    let resp = req.send().await.expect("ingest request");
    assert!(
        resp.status().is_success(),
        "ingest failed ({}): {}",
        resp.status(),
        resp.text().await.unwrap_or_default(),
    );

    let handle = handle_of_only_profile(&app.pool).await;
    latest_emitter_for(&app.pool, &handle).await
}

/// The bug this task fixes: `temper-cli` in cloud mode now lands `<handle>@cli`, not `@web`.
/// `app.client` is built through `build_client_from` with `Surface::CliCloud` — it is the
/// CLI's own client, not a hand-rolled imitation.
#[sqlx::test(migrations = "../../migrations")]
async fn cli_client_write_lands_at_cli_emitter(pool: PgPool) {
    let app = common::setup(pool).await;

    app.client
        .resources()
        .create(&common::probe_create_request())
        .await
        .expect("create via the CLI's own client");

    let handle = handle_of_only_profile(&app.pool).await;
    assert_eq!(
        latest_emitter_for(&app.pool, &handle).await,
        format!("{handle}@{}", Surface::CliCloud.marker()),
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn sdk_header_lands_at_sdk_emitter(pool: PgPool) {
    let app = common::setup(pool).await;
    let handle = handle_of_only_profile(&app.pool).await;
    let emitter = create_and_read_emitter(&app, Some("sdk")).await;
    assert_eq!(emitter, format!("{handle}@sdk"));
}

/// A browser sends no such header. It still lands `@web` — this is the no-regression case.
#[sqlx::test(migrations = "../../migrations")]
async fn absent_header_lands_at_web_emitter(pool: PgPool) {
    let app = common::setup(pool).await;
    let handle = handle_of_only_profile(&app.pool).await;
    let emitter = create_and_read_emitter(&app, None).await;
    assert_eq!(emitter, format!("{handle}@web"));
}

/// The deny direction. `mcp` is untrusted by construction (`temper-mcp` never crosses this
/// boundary), garbage is untrusted, empty is untrusted. All three degrade to `web`, and — the
/// load-bearing half — none of them 500s. Surface is provenance, never authorization.
#[sqlx::test(migrations = "../../migrations")]
async fn untrusted_headers_degrade_to_web_without_failing(pool: PgPool) {
    let app = common::setup(pool).await;
    let handle = handle_of_only_profile(&app.pool).await;

    for claimed in ["mcp", "not-a-surface", "", "CLI", "sdk; drop table"] {
        let emitter = create_and_read_emitter(&app, Some(claimed)).await;
        assert_eq!(
            emitter,
            format!("{handle}@web"),
            "claimed surface {claimed:?} should have degraded to web",
        );
    }
}
```

Add `probe_create_request()` to `tests/e2e/tests/common/mod.rs`, returning whatever request type `app.client.resources().create(..)` takes. Read `crates/temper-client/src/resources.rs` for the exact signature and `temper_workflow::types::resource::ResourceCreateRequest` for the field set, then build a minimal valid instance targeting `@me/temper`. Do **not** guess the field names — read the struct.

The `#[sqlx::test]` attribute, the migrations path, and the `common::setup` idiom must match the other files in `tests/e2e/tests/`. Read `sdk_emitter_entity_e2e.rs` and copy its exact attribute form rather than trusting the one written above.

- [ ] **Step 3: Run tests to verify they fail on the pre-change binary**

This checks the tests are real. Stash the server-side change, run, unstash:

```bash
git stash push -- crates/temper-api/src/handlers
cargo test -p temper-e2e --features test-db --test surface_attribution_e2e -- cli_client_write_lands_at_cli_emitter
git stash pop
```

Expected: FAIL — `assertion failed: left: "<handle>@web", right: "<handle>@cli"`. That literal string is the bug, reproduced.

(Use `cargo test --test <name>`, not `cargo nextest`: a freshly built e2e binary hangs at nextest's `--list` step on macOS. Confirm the e2e crate's package name with `rg -n '^name' tests/e2e/Cargo.toml` — it may not be `temper-e2e`.)

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo build -p temper-cli --bin temper   # nextest/e2e spawn a stale binary otherwise
cargo test -p temper-e2e --features test-db --test surface_attribution_e2e
```

Expected: PASS — four tests.

- [ ] **Step 5: Discharge the deferral note**

In `tests/e2e/tests/sdk_emitter_entity_e2e.rs`, replace the module-doc line:

> The wire-level assertion (an `X-Temper-Surface: sdk` request attributing to `<handle>@sdk`) belongs to P2 — no client can send the marker yet. What P1 owes is that the entity resolves.

with:

```rust
//! The wire-level assertion (an `X-Temper-Surface: sdk` request attributing to `<handle>@sdk`)
//! landed in P2 and lives in `surface_attribution_e2e.rs`. What this file owes is that the
//! entity resolves at all.
```

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add tests/e2e
git commit -m "P2: e2e proof that cli/sdk/web attribution survives the wire"
```

---

### Task 8: Full verification

**Files:** none.

- [ ] **Step 1: Offline check — the honest local probe**

`cargo make` tasks force `SQLX_OFFLINE=true`, so this is what catches a missing `.sqlx` cache entry. No SQL changed in this plan, so no `cargo sqlx prepare` ritual should be needed; if this step reports a missing query, something unplanned touched SQL — stop and report.

Run: `cargo make check > /tmp/p2-check.log 2>&1; tail -30 /tmp/p2-check.log`
Expected: clean fmt, clippy, docs, machete.

- [ ] **Step 2: Unit tests**

Run: `cargo make test > /tmp/p2-test.log 2>&1; tail -20 /tmp/p2-test.log`
Expected: PASS.

- [ ] **Step 3: Database tests**

Run: `cargo make docker-up` then `cargo make test-db > /tmp/p2-testdb.log 2>&1; tail -20 /tmp/p2-testdb.log`
Expected: PASS.

- [ ] **Step 4: E2E**

Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e > /tmp/p2-e2e.log 2>&1; tail -20 /tmp/p2-e2e.log`
Expected: PASS.

This change alters the wire format of every client request, and `test-e2e` alone does not enable `test-embed`. Also run:

Run: `cargo make test-e2e-embed > /tmp/p2-e2e-embed.log 2>&1; tail -20 /tmp/p2-e2e-embed.log`
Expected: PASS.

- [ ] **Step 5: The contract gate**

Run: `bash .github/scripts/check-openapi-spec.sh`
Expected: `openapi.json is up to date with the router`.

- [ ] **Step 6: Confirm nothing still hardcodes the surface**

Run: `rg -n "Surface::ApiHttp" crates/temper-api/src/`
Expected: no matches.

- [ ] **Step 7: Push and open the PR**

Branch: `jct/p2-surface-over-the-wire`. Merge `origin/main` first — CI runs `pull/<n>/merge`.

```bash
git fetch origin && git merge origin/main
git push -u origin jct/p2-surface-over-the-wire
```

The PR description **must** contain the callout, in its own section:

> **This visibly changes ledger attribution for existing CLI writes from `<handle>@web` to `<handle>@cli`.** That is a correction, not a regression: the `@cli` emitter entity has been provisioned for every profile since inception and never once resolved, because `temper-cli` dropped its `Surface` at the HTTP boundary. Event history written before this PR stays attributed to `@web`; events after it are attributed honestly. Nothing is rewritten.

It must also note that `/api/*` paths gained an `X-Temper-Surface` parameter in `openapi.json`, and that P1's migration (`20260709000030`) is already deployed, so an `sdk` claim cannot 500 on a missing emitter.

---

## Notes for the implementer

- **Do not add a migration.** A sibling session owns `kb_entities(profile_id, name)` uniqueness and is authoring migrations concurrently.
- **`temper-mcp` needs no change.** It reaches `DbBackend` in-process with `Surface::Mcp` and never crosses this header boundary — which is exactly why `mcp` is not on the allowlist.
- **After merge**, reinstall the CLI so your local `temper` sends the header: `cargo install --path crates/temper-cli`.
- The `origin` field on operations commands is *not* dead after this change. In-process callers (MCP, and `temper-api`'s own handlers) read `cmd.origin`. Only `temper-cli`'s HTTP-forwarding backend ignores it, because for that backend the surface travels in the header instead.
