# Local Integration and E2E Testing Infrastructure

## Problem

The temper project has unit tests and DB-backed integration tests (via `#[sqlx::test]`), but no
infrastructure for testing the CLI and API together end-to-end. Changes like structured logging
that affect observable output across the full request pipeline can't be verified automatically.

Additionally, config loading is hardwired to disk I/O throughout `temper-client` and `temper-cli`,
making it impossible to construct isolated test environments without env var manipulation. The
auth.json path has no override mechanism at all.

## Goals

1. **Config injection** — every config/client construction path gets a `_from` variant that accepts
   explicit values, while existing disk-reading convenience functions remain unchanged
2. **E2E test harness** — a top-level `tests/e2e/` crate that exercises CLI-as-library against an
   in-process API server with full DB isolation
3. **Tracing verification** — a test `tracing::Layer` that captures spans/events for assertion
4. **Initial test coverage** — import, search, auth rejection, and structured logging verification
5. **Zero config leakage** — e2e tests never read `~/.config/temper/config.toml` or `auth.json`

## Non-Goals

- CLI `--config <path>` flag (follow-up work)
- Full CLI command coverage beyond the four initial test cases
- CI pipeline integration (revisit at end of feature branch)
- Subprocess/binary testing (library calls only for now)

## Design

### 1. `build_client_from` in temper-client

**File:** `crates/temper-client/src/config.rs`

Add a config-injected client builder alongside the existing disk-reading one:

```rust
/// Build client from explicit config and optional stored auth (no disk reads).
pub fn build_client_from(
    config: &TemperConfig,
    auth: Option<&StoredAuth>,
) -> Result<TemperClient> {
    let url = api_url(config);
    let device_id = auth.and_then(|a| a.device_id.clone());
    let mut client = TemperClient::new(&url, device_id);
    match oauth_config(config) {
        Ok(oauth) => {
            client = client.with_oauth(oauth);
        }
        Err(e) => {
            tracing::debug!("OAuth config not available: {e}");
        }
    }
    Ok(client)
}
```

The existing `build_client()` delegates to `build_client_from` internally:

```rust
pub fn build_client() -> Result<TemperClient> {
    let config = load_cloud_config()?;
    let auth = crate::auth::load_auth().ok().flatten();
    build_client_from(&config, auth.as_ref())
}
```

**Rationale:** `StoredAuth` is already path-parameterized (`load_auth_from`, `save_auth_to`).
The missing piece is `build_client` accepting these values instead of reading from disk.
`api_url()` still checks `TEMPER_API_URL` env var, but tests using `build_client_from` will
pass a config with the test server URL in `cloud.api_url`, so the env var is irrelevant.

### 2. `load_from` in temper-cli

**File:** `crates/temper-cli/src/config.rs`

Add a config-injected variant of `load()`:

```rust
/// Build Config from an explicit TemperConfig + vault override (no disk reads).
pub fn load_from(global: &TemperConfig, cli_vault: Option<&str>) -> Config {
    let vault_root = cli_vault
        .map(|v| expand_tilde(v))
        .unwrap_or_else(|| expand_tilde(&global.vault.path));
    Config {
        state_dir: vault_root.join(".temper"),
        vault_root,
        contexts: global.sync.subscriptions.contexts.clone(),
        skill_output: expand_tilde(&global.skill.output),
        skill_framework: global.skill.framework.clone(),
    }
}
```

The existing `load()` can optionally delegate to `load_from` internally (minor refactor,
not required for this task).

### 3. CLI runtime `_from` variants

**File:** `crates/temper-cli/src/actions/runtime.rs`

Add variants that accept a pre-built client:

```rust
/// Execute with a pre-built client (for e2e tests).
pub fn with_client_from<F, T>(client: temper_client::TemperClient, f: F) -> Result<T>
where
    F: FnOnce(&temper_client::TemperClient) -> Pin<Box<dyn Future<Output = Result<T>> + '_>>,
{
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    rt.block_on(f(&client))
}
```

### 4. E2E test crate structure

```
tests/e2e/
├── Cargo.toml
└── tests/
    ├── common/
    │   ├── mod.rs          # E2eTestApp — server lifecycle, config, client
    │   └── tracing.rs      # TestTracingLayer for log assertions
    ├── import_test.rs      # CLI import → API → verify resource created
    ├── search_test.rs      # CLI search → API → verify results returned
    ├── logging_test.rs     # Verify tracing spans have expected fields
    └── auth_test.rs        # Missing/bad token → 401
```

**Cargo.toml dependencies:**
- `temper-api` (for `setup_test_app`, `create_app`, `AppState`, `MIGRATOR`)
- `temper-cli` (for CLI command functions as library calls)
- `temper-client` (for `build_client_from`, `StoredAuth`)
- `temper-core` (for `TemperConfig`)
- `sqlx` with `migrate` feature
- `tokio`, `reqwest`, `tracing`, `tracing-subscriber`
- `tempfile` (for temp vault directory)

**Feature gating:** `test-db` feature, same as existing API tests.

### 5. `E2eTestApp` — the test harness

**File:** `tests/e2e/tests/common/mod.rs`

Combines the existing `setup_test_app` pattern with config injection:

```rust
pub struct E2eTestApp {
    pub addr: SocketAddr,
    pub pool: PgPool,
    pub client: TemperClient,           // pointed at test server
    pub config: TemperConfig,           // in-memory, no disk
    pub cli_config: temper_cli::config::Config, // vault points to tempdir
    pub token: String,                  // valid test JWT
    pub vault_dir: TempDir,            // isolated temp vault
}

impl E2eTestApp {
    pub async fn setup(pool: PgPool) -> Self {
        // 1. Seed fixtures (reuse temper-api's clean_and_seed)
        // 2. Build JwksKeyStore with test RSA public key
        // 3. Build ApiConfig with auth_issuer="test-issuer", port=0
        // 4. Create Axum app, bind to 127.0.0.1:0
        // 5. Spawn server in background task
        // 6. Build TemperConfig with cloud.api_url = "http://127.0.0.1:{port}"
        // 7. Create temp vault directory with .temper/ structure
        // 8. Build CLI Config via load_from(config, vault_dir)
        // 9. Build TemperClient via build_client_from(config, test_auth)
        // 10. Generate test JWT
    }

    pub fn base_url(&self) -> String { ... }
    pub fn url(&self, path: &str) -> String { ... }
}
```

**Key isolation guarantees:**
- `TemperConfig` constructed in-memory — never reads `~/.config/temper/config.toml`
- `cloud.api_url` points to `127.0.0.1:{random_port}` — never hits production
- `StoredAuth` constructed in-memory — never reads `~/.config/temper/auth.json`
- Vault directory is a `TempDir` — never touches the real vault
- Each test gets its own DB via `#[sqlx::test]`

### 6. Test tracing layer

**File:** `tests/e2e/tests/common/tracing.rs`

A `tracing::Layer` that collects events and span data for assertions:

```rust
pub struct TestTracingLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

pub struct CapturedEvent {
    pub target: String,
    pub level: tracing::Level,
    pub fields: HashMap<String, String>,
    pub span_fields: HashMap<String, String>,
}

impl TestTracingLayer {
    pub fn new() -> (Self, Arc<Mutex<Vec<CapturedEvent>>>) { ... }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for TestTracingLayer {
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Capture event fields + walk parent spans to collect span fields
    }
}
```

Tests assert on captured events after making requests:

```rust
// Verify a GET /api/resources produces a span with expected fields
let events = captured.lock().unwrap();
assert!(events.iter().any(|e|
    e.span_fields.get("method") == Some(&"GET".into()) &&
    e.span_fields.get("path") == Some(&"/api/resources".into()) &&
    e.span_fields.contains_key("profile_id")
));
```

### 7. Initial test cases

**import_test.rs** — `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`
1. Set up `E2eTestApp`
2. Call the CLI import function with a test markdown file in the temp vault
3. Assert: resource created in DB, accessible via API

**search_test.rs** — `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`
1. Set up `E2eTestApp`
2. Create a resource via API (seed data or direct insert)
3. Call the CLI search function
4. Assert: search results contain the seeded resource

**auth_test.rs** — `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`
1. Set up `E2eTestApp`
2. Build a client with no auth token
3. Call a protected endpoint
4. Assert: 401 response
5. Build a client with an expired JWT
6. Assert: 401 response

**logging_test.rs** — `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`
1. Set up `E2eTestApp` with `TestTracingLayer` installed
2. Make a request to `/api/resources`
3. Assert: captured spans contain `method`, `path`, `status`, `profile_id`
4. Make a request without auth
5. Assert: captured events contain error-level log with `status_code=401`

### 8. cargo-make integration

**File:** `tools/cargo-make/main.toml` (or root `Makefile.toml`)

```toml
[tasks.test-e2e]
description = "Run E2E tests (requires Docker Postgres)"
command = "cargo"
args = ["nextest", "run", "--manifest-path", "tests/e2e/Cargo.toml", "--features", "test-db"]
dependencies = ["docker-up"]
```

Update `test-all` to include e2e:

```toml
[tasks.test-all]
dependencies = ["test-all-rust", "test-all-ts", "test-e2e"]
```

### 9. Test execution constraints

- Tests run serially (inherent to `#[sqlx::test]` — each gets its own DB)
- `test-db` feature gate prevents accidental execution without Docker Postgres
- `cargo nextest` is the runner (already configured in `.config/nextest.toml`)

## Files Modified

| File | Change |
|------|--------|
| `crates/temper-client/src/config.rs` | Add `build_client_from()`, refactor `build_client()` to delegate |
| `crates/temper-cli/src/config.rs` | Add `load_from()` |
| `crates/temper-cli/src/actions/runtime.rs` | Add `with_client_from()` |
| `tests/e2e/Cargo.toml` | New crate |
| `tests/e2e/tests/common/mod.rs` | `E2eTestApp` harness |
| `tests/e2e/tests/common/tracing.rs` | `TestTracingLayer` |
| `tests/e2e/tests/import_test.rs` | Import flow e2e test |
| `tests/e2e/tests/search_test.rs` | Search flow e2e test |
| `tests/e2e/tests/auth_test.rs` | Auth rejection e2e test |
| `tests/e2e/tests/logging_test.rs` | Structured logging verification |
| `Makefile.toml` or `tools/cargo-make/main.toml` | `test-e2e` task |

## Cross-Crate Test Asset Access

The e2e crate needs test RSA keys and fixture logic currently in `crates/temper-api/tests/common/`.
These are not part of `temper-api`'s public API — they're test-only code.

**Approach:** The e2e crate duplicates the minimal set needed:
- RSA key files copied to `tests/e2e/tests/fixtures/` (or referenced via relative path constant)
- `JwksKeyStore` construction and `generate_test_jwt` reimplemented in `tests/e2e/tests/common/`
  using the same key material
- Fixture seeding calls `temper_api`'s fixture module if it's re-exported, otherwise reimplements
  `clean_and_seed` (it's ~20 lines)

This avoids coupling the e2e crate to temper-api's internal test structure. If temper-api later
exports a `test-support` feature with these helpers, the e2e crate can adopt it.

## Files Not Modified

- `crates/temper-api/src/` — no changes to the API server
- `crates/temper-core/src/types/config.rs` — `TemperConfig` already has `Default` and all needed types

## Risk: Config Leakage

The primary risk this design addresses. Summary of isolation guarantees:

| Config Source | Production Path | E2E Test Path |
|---------------|----------------|---------------|
| `config.toml` | `load_config()` → disk | `TemperConfig::default()` + overrides in-memory |
| `auth.json` | `load_auth()` → disk | `StoredAuth` constructed in-memory |
| API URL | `cloud.api_url` from file | `cloud.api_url = "http://127.0.0.1:{port}"` |
| Vault path | `[vault].path` from file | `TempDir` path |
| Device ID | `auth.json` `device_id` field | Test UUID in `StoredAuth` |

No env vars are set or read during e2e tests. The `_from` pattern makes isolation structural,
not environmental.

## Follow-Up Work (Not in Scope)

- CLI `--config <path>` flag for all commands
- Full CLI command coverage (all subcommands exercised in e2e)
- CI pipeline integration with `test-e2e` in GitHub Actions
- Subprocess smoke test (build binary, run against test server)
