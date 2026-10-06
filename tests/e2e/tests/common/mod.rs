#![allow(dead_code)]
//! Shared e2e test infrastructure.
//!
//! `E2eTestApp` starts an in-process Axum server backed by an isolated
//! per-test database and builds a `TemperClient` with injected config
//! (no disk reads, no env var manipulation).

pub mod tracing_layer;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::net::TcpListener;

use temper_api::create_app;
use temper_client::auth::{MemoryTokenStore, Provider, StoredAuth};
use temper_core::types::config::{CloudSection, CloudVaultConfig, TemperConfig};
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::{
    config::ApiConfig,
    state::{AppState, JwksKeyStore},
};

/// A running e2e test environment with in-process API server and injected client.
/// Holds the serve task alive for the app's life and aborts it on drop. Without this,
/// every `#[sqlx::test]`'s listener and serve task outlived its test and its database
/// — a bound loopback socket per test for the binary's whole run (found in the
/// arc-boundary review, 2026-09-24; the door's canary probe already aborts, the
/// shared harness did not).
pub struct ServeGuard(tokio::task::JoinHandle<()>);

impl Drop for ServeGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct E2eTestApp {
    pub addr: SocketAddr,
    pub pool: PgPool,
    pub client: temper_client::TemperClient,
    pub reqwest_client: reqwest::Client,
    pub config: TemperConfig,
    pub cli_config: temper_cli::config::Config,
    pub token: String,
    pub vault_dir: TempDir,
    pub _serve: ServeGuard,
}

impl E2eTestApp {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }

    /// Request parts whose `BearerToken` is THIS app's real minted JWT — the same
    /// token `client` presents, so the API's own auth middleware resolves it against
    /// the profile `approve_app_principal` provisioned. The synthetic tokens and
    /// seeded claims the suites used to build never survive the wire: anything
    /// forwarded from these parts is one hop the API fully adjudicates itself.
    pub fn relay_parts(&self) -> axum::http::request::Parts {
        self.relay_parts_for(&self.token)
    }

    /// Request parts in the full production shape: both extensions the JWT middleware
    /// injects (`RawJwtClaims` + `BearerToken`) naming this app's principal. Until the
    /// network door's teardown an in-process gate read the claims; nothing does now —
    /// every tool forwards the bearer alone — so the claims ride for fidelity. (Contrast
    /// [`Self::relay_parts`]: the bearer only.)
    pub fn direct_parts(&self) -> axum::http::request::Parts {
        axum::http::Request::builder()
            .extension(temper_mcp_server::BearerToken(self.token.clone()))
            .extension(temper_services::auth::RawJwtClaims {
                sub: "e2e-test-user".to_string(),
                email: None,
                email_verified: None,
                azp: None,
                gty: None,
                exp: (Utc::now() + Duration::hours(1)).timestamp(),
                iat: 0,
            })
            .body(())
            .expect("direct parts build")
            .into_parts()
            .0
    }

    /// [`Self::relay_parts`] for an ARBITRARY minted token — the second identity of
    /// an owner/other test. Identity rides the bearer alone, so both identities can
    /// share the one MCP service.
    pub fn relay_parts_for(&self, token: &str) -> axum::http::request::Parts {
        axum::http::Request::builder()
            .extension(temper_mcp_server::BearerToken(token.to_string()))
            .body(())
            .expect("relay parts build")
            .into_parts()
            .0
    }

    /// The deployed relay pointed at THIS app's real listener, built by the shell's own reader
    /// (`temper_mcp_server::config::deployed_relay`) from the two variables the deployment sets —
    /// so the harness relays with exactly the production seam (the edge-verified bearer,
    /// `Surface::Mcp`, the service credential and the `mcp` carrier). The listener is BOUND
    /// HERE, in the test process — temper-mcp never constructs one (the §8 trap).
    pub fn mcp_deployed_relay(&self) -> temper_mcp_server::config::DeployedRelay {
        deployed_relay_to(&self.base_url())
    }

    /// The MCP service every suite drives: the deployed door's tool service
    /// (`temper_mcp_server::tool_service`, the one `build_router` mounts) relaying to this app's
    /// listener, with the blob door closed, as on a blob-less deployment. The service holds no
    /// database pool and carries NO auth state: every tool forwards the bearer in the parts it is
    /// handed, so each call acts as its own principal — the same path production dispatch takes.
    pub async fn mcp_relay_service(&self) -> temper_mcp::service::TemperMcpService {
        temper_mcp_server::tool_service(
            temper_mcp_server::config::blob_door(None, false),
            self.mcp_deployed_relay(),
        )
    }

    /// The relay service with the blob door OPEN — the blob families' harness (beat G4
    /// parity suite). The MCP side holds only the door's posture and its single-request
    /// ceiling (the tool layer holds no store and no pool); the blobs themselves live in
    /// the app listener's store, which the caller shares with the test
    /// (`setup_with_blob_store_shared`) so a test can seed blobs directly where a commit
    /// gate would get in the way. `single_request_max_bytes` is deliberately small so the
    /// read-ceiling refusal is cheap to construct; the ceiling's own number is the
    /// operator's knob the tool names verbatim.
    pub async fn mcp_relay_service_with_blob(
        &self,
        single_request_max_bytes: usize,
    ) -> temper_mcp::service::TemperMcpService {
        temper_mcp_server::tool_service(
            temper_mcp::BlobDoor::Open {
                single_request_max_bytes,
            },
            self.mcp_deployed_relay(),
        )
    }
}

/// Resolve the path to the compiled `temper` binary.
///
/// `CARGO_BIN_EXE_temper` is only set by cargo for integration tests in the
/// same package that declares the binary. For this e2e crate (which lists
/// `temper-cli` as a dev-dependency), we derive the path from the *running
/// test executable* instead: the test binary and the `temper` binary are built
/// into the same target directory, so resolving relative to `current_exe()` is
/// robust to relocated target dirs. In particular `cargo llvm-cov` builds into
/// `target/llvm-cov-target/` rather than `target/`, which a hard-coded
/// `<workspace>/target/<profile>/` path would miss (the CLI-spawning e2e tests
/// then fail with `NotFound` under coverage).
///
/// # It runs whatever binary is already there — which `-p temper-e2e` does not rebuild
///
/// Nothing in this resolution *builds* `temper`. `cargo nextest run -p temper-e2e` builds this
/// crate's test targets and its dependencies as libraries; the `temper` **bin target** is not
/// among them, so a scoped local run happily executes a binary from some earlier build while the
/// source says something else. Every assertion in this file that reads a `temper` process's
/// stdout or stderr is then testing code that is not in the working tree.
///
/// This was **witnessed, not theorised**: a deliberate bite probe that reverted a fix in
/// `warmup.rs` still passed the test written to catch it, because the 24-minute-old binary on
/// disk still carried the fix. A green run proved nothing about the change under test.
///
/// CI is unaffected — it runs `cargo nextest run --workspace`, which builds every bin target —
/// so this is a trap for scoped local runs only. **Before trusting a local run of these tests,
/// and always before a bite probe, `cargo build -p temper-cli --bin temper`.**
///
/// The test executable lives in `<target>/<profile>/deps/`; the `temper` binary
/// is one level up in `<target>/<profile>/`.
fn temper_bin_path() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("current_exe() for the running test binary");
    path.pop(); // drop the test-binary filename → .../deps
    if path.ends_with("deps") {
        path.pop(); // → .../<profile>
    }
    path.join("temper")
}

/// Run the `temper` CLI binary against the in-process Axum server.
///
/// The CLI's `load_global_config` requires the global config file to
/// exist (unlike `temper-core::load_config_from` which returns defaults
/// when absent). In CI the runner has no `~/.config/temper/config.toml`,
/// so this helper materializes the test app's `TemperConfig` to a
/// temp TOML file and points `TEMPER_GLOBAL_CONFIG` at it.
///
/// Sets `TEMPER_API_URL` to the test server's URL and `TEMPER_TOKEN`
/// to the test JWT so the CLI hits the real handler stack without
/// needing a separate auth round-trip. Spawned via `spawn_blocking`
/// so we don't block the runtime.
///
/// Verified env-var names against `crates/temper-client/src/config.rs`
/// (`TEMPER_API_URL`), `crates/temper-cli/src/actions/runtime.rs`
/// (`TEMPER_TOKEN`), and `crates/temper-core/src/types/config.rs`
/// (`TEMPER_GLOBAL_CONFIG`).
pub async fn run_temper_cli(
    app: &E2eTestApp,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    run_temper_cli_with_env(app, &[], args).await
}

/// `run_temper_cli`, with `input` piped to the spawned process's stdin.
///
/// For the commands whose input arrives on stdin rather than in a flag — `resource update`'s body,
/// `query`'s plan. Asserting those through the real binary is the only way to exercise the
/// non-TTY auto-detect branch, which no in-process test can reach.
pub async fn run_temper_cli_with_stdin(
    app: &E2eTestApp,
    input: &str,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config TOML");
    spawn_temper(
        &app.base_url(),
        &app.token,
        &config_path,
        &[],
        args,
        Some(input),
    )
    .await
}

/// `run_temper_cli`, plus extra environment for the spawned process.
///
/// Exists for the cases where the *environment* is the thing under test rather than the command —
/// `RUST_LOG`, say, when asserting which stream the CLI's logs land on.
pub async fn run_temper_cli_with_env(
    app: &E2eTestApp,
    env: &[(&str, &str)],
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    // Materialize the test TemperConfig to a TOML file inside the test's
    // vault temp directory so the spawned CLI can read it. The path lives
    // alongside the vault projection so it shares the test's cleanup.
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("test-temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config for CLI invocation");

    spawn_temper(&app.base_url(), &app.token, &config_path, env, args, None).await
}

/// Run the real `temper` binary against an arbitrary API URL and token.
///
/// `run_temper_cli` is the convenience wrapper for `E2eTestApp` (it materializes
/// the config file and delegates here); this is the form harnesses with their
/// own app struct (e.g. `SlackLinkApp`, which has no `token`/`config`/`vault_dir`)
/// can use directly. The caller is responsible for the config file at
/// `config_path` existing and pointing `TEMPER_GLOBAL_CONFIG` at something the
/// CLI's `load_global_config` can read.
pub async fn run_temper_cli_with_token(
    api_url: &str,
    token: &str,
    config_path: &std::path::Path,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    spawn_temper(api_url, token, config_path, &[], args, None).await
}

/// The one place the `temper` binary is actually spawned. Every helper above funnels here so the
/// env-var contract (`TEMPER_API_URL` / `TEMPER_TOKEN` / `TEMPER_GLOBAL_CONFIG`) has a single
/// definition rather than one per convenience wrapper.
async fn spawn_temper(
    api_url: &str,
    token: &str,
    config_path: &std::path::Path,
    env: &[(&str, &str)],
    args: &[&str],
    stdin: Option<&str>,
) -> std::io::Result<std::process::Output> {
    let bin = temper_bin_path();
    let url = api_url.to_string();
    let token = token.to_string();
    let config_path = config_path.to_path_buf();
    let args_owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let stdin_owned: Option<String> = stdin.map(ToOwned::to_owned);
    let env_owned: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();

    tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&bin);
        cmd.env("TEMPER_API_URL", &url)
            .env("TEMPER_TOKEN", &token)
            .env("TEMPER_GLOBAL_CONFIG", &config_path)
            .args(&args_owned);
        for (key, value) in &env_owned {
            cmd.env(key, value);
        }
        // `Command::output()` defaults stdin to `Stdio::null()` — a non-TTY that is immediately at
        // EOF, which is what makes the no-stdin case well-defined rather than a hang. Piping is
        // opt-in, and goes through this one spawn site so the env-var contract stays single-defined.
        let Some(input) = stdin_owned else {
            return cmd.output();
        };
        use std::io::Write as _;
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn()?;
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(input.as_bytes())?;
        child.wait_with_output()
    })
    .await
    .expect("spawn_blocking join")
}

/// The audience this test instance validates. Every test JWT must carry it as its `aud`.
///
/// Before the auth-config work the fixtures set `auth_audience: None`, which set
/// `validate_aud = false` — so these tokens carried no `aud` at all and the e2e suite never
/// exercised audience validation on either surface. It does now.
pub const TEST_AUDIENCE: &str = "test-audience";

/// The deployed relay to `api_base_url`, read by the shell's own reader from the two variables a
/// deployment sets (`TEMPER_API_BASE_URL`, `TEMPER_MCP_SERVICE_SECRET` = [`TEST_MCP_SERVICE_SECRET`]).
pub fn deployed_relay_to(api_base_url: &str) -> temper_mcp_server::config::DeployedRelay {
    let api_base_url = api_base_url.to_string();
    temper_mcp_server::config::deployed_relay(&move |key: &str| match key {
        "TEMPER_API_BASE_URL" => Some(api_base_url.clone()),
        "TEMPER_MCP_SERVICE_SECRET" => Some(TEST_MCP_SERVICE_SECRET.to_string()),
        _ => None,
    })
}

/// The router-level suites' discovery config: no client id (registration answers 503), the
/// compiled-in loopback rule on, and the given public base URL.
pub fn mcp_discovery_config(mcp_base_url: &str) -> temper_mcp_server::DiscoveryConfig {
    temper_mcp_server::DiscoveryConfig {
        mcp_base_url: mcp_base_url.to_string(),
        mcp_client_id: None,
        oauth: temper_mcp_server::discovery_config::OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: true,
        },
    }
}

/// [`mcp_server_config`] relaying to `api_base_url` through the deployed seam.
pub fn mcp_server_config_relaying(
    blob_door: temper_mcp::BlobDoor,
    api_base_url: &str,
) -> temper_mcp_server::McpServerConfig {
    temper_mcp_server::McpServerConfig {
        relay: deployed_relay_to(api_base_url),
        ..mcp_server_config(blob_door)
    }
}

/// The MCP server's boot config for the router-level suites: the same auth identity the harness
/// API validates (issuer `test-issuer`, [`TEST_AUDIENCE`]), no CORS origins, and the given blob
/// door. No database URL and no pool — the deployed edge holds neither. The relay is unset (the
/// tool door is dark, answering the deployment's own sentence); [`mcp_server_config_relaying`]
/// sets it.
pub fn mcp_server_config(blob_door: temper_mcp::BlobDoor) -> temper_mcp_server::McpServerConfig {
    temper_mcp_server::McpServerConfig {
        auth: temper_auth::config::AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: TEST_AUDIENCE.to_string(),
            mcp_audience: TEST_AUDIENCE.to_string(),
            mode: temper_auth::config::AuthMode::ExternalIdp,
        },
        cors_origins: vec![],
        blob_door,
        relay: temper_mcp_server::config::deployed_relay(&|_: &str| None),
    }
}

/// The JWT edge's key store over the harness's RSA test key, so the MCP router verifies the
/// tokens [`generate_test_jwt`] mints.
pub fn mcp_test_jwks() -> JwksKeyStore {
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.pub"))
            .expect("decoding key");
    JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256)
}

/// The service credential the relay harness configures on BOTH sides of the network
/// door: the API's relay-trust middleware validates it constant-time, and the MCP
/// relay presents it as `X-Temper-Service-Credential`. A test constant — production
/// keeps this in the operator's environment (`TEMPER_MCP_SERVICE_SECRET`,
/// injectable, §D6); the value exists so the harness can pin that the two sides
/// must agree and that the API refuses any other.
pub const TEST_MCP_SERVICE_SECRET: &str = "e2e-mcp-relay-service-credential";

/// JWT claims for test tokens.
#[derive(Debug, Serialize, Deserialize)]
struct TestClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// Claims with **no `aud` at all** — the subtler hole.
///
/// Setting an expected audience is not enough on its own: `jsonwebtoken` only checks `aud` when the
/// claim is PRESENT (`required_spec_claims` defaults to `{"exp"}`, and its docs say so outright).
/// So a token omitting `aud` entirely was accepted even with `validate_aud = true` — and no test in
/// this suite could catch it, because every fixture token carries an `aud`.
#[derive(Debug, Serialize, Deserialize)]
struct NoAudienceClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: String,
    iat: i64,
    exp: i64,
}

/// Sign a JWT that omits the `aud` claim entirely. Correctly signed, unexpired, right issuer.
pub fn generate_jwt_without_audience(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = NoAudienceClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign aud-less JWT")
}

/// Sign a JWT for a DIFFERENT audience than this instance validates — a token the same trusted
/// issuer minted for some other API. Correctly-signed, unexpired, right issuer, wrong `aud`.
///
/// This is the token that used to sail straight through: an unset `AUTH_AUDIENCE` set
/// `validate_aud = false`, so temper-api accepted it. Anything using this helper is asserting a
/// refusal.
pub fn generate_jwt_for_other_audience(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        aud: "https://some-other-api.example/api".to_string(),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign foreign-audience JWT")
}

/// Sign a JWT with the test RSA private key. Valid for 1 hour.
pub fn generate_test_jwt(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        aud: TEST_AUDIENCE.to_string(),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign test JWT")
}

/// JWT claims for a machine (`client_credentials`) test token. `gty` is the definitive
/// machine signal `auth::classify` keys on; `azp` carries the client id. No email:
/// a machine has none.
#[derive(Debug, Serialize, Deserialize)]
struct MachineTestClaims {
    sub: String,
    azp: String,
    gty: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// Sign a machine JWT with the test RSA private key. Valid for 1 hour. The claim shape
/// mirrors the real Auth0 `client_credentials` token pinned by `normalize.rs`'s
/// known-answer test.
pub fn generate_machine_jwt(client_id: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = MachineTestClaims {
        sub: format!("{client_id}@clients"),
        azp: client_id.to_string(),
        gty: "client-credentials".to_string(),
        iss: "test-issuer".to_string(),
        aud: TEST_AUDIENCE.to_string(),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign machine JWT")
}

/// Sign an expired JWT (expired 1 hour ago).
pub fn generate_expired_jwt(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        aud: TEST_AUDIENCE.to_string(),
        iat: now - 7200,
        exp: now - 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign expired JWT")
}

/// Generate a JWT for a second test user (distinct from the primary e2e user).
pub fn generate_second_user_jwt() -> String {
    generate_test_jwt("e2e-second-user", "second@test.example.com")
}

/// Generate a JWT for a third test user (distinct from the primary and second e2e users).
pub fn generate_third_user_jwt() -> String {
    generate_test_jwt("e2e-third-user", "third@test.example.com")
}

/// Sign a JWT with the test Ed25519 private key (EdDSA). Valid for 1 hour.
///
/// Mirrors `generate_test_jwt` exactly (same claims shape, same issuer) but
/// signs with `Algorithm::EdDSA` against the `test_ed25519.pkcs8` fixture,
/// proving the algorithm-aware verification path added in Task 0.1.
pub fn generate_test_jwt_eddsa(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_ed_pem(include_bytes!("../fixtures/test_ed25519.pkcs8"))
        .expect("Failed to load test Ed25519 private key");

    let now = Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        aud: TEST_AUDIENCE.to_string(),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &encoding_key)
        .expect("Failed to sign test JWT")
}

/// Enable invite-only mode in tests by making the admin an `owner` of the
/// `temper-system` gating team and flipping the setting.
///
/// `temper-system` is seeded by the L0 kernel migration (`20260625000001`) and,
/// since the auto-join generalization (`20260629000002`), is flagged as an
/// auto-join team — so in `open` mode (the default before this call) the admin
/// profile has ALREADY been auto-joined as a `watcher`. The owner write must
/// therefore UPSERT (`DO UPDATE SET role`), promoting the existing watcher row
/// to `owner`; a plain `DO NOTHING` would leave the admin a watcher and
/// `is_system_admin` would stay false. This mirrors the production root step
/// (the L0 content-delivery guide grants owner via `ON CONFLICT DO UPDATE`).
/// The `kb_teams` upsert by slug tolerates the row already existing; access
/// predicates resolve the gating team by `slug = gating_team_slug`.
pub async fn enable_invite_only(pool: &PgPool, admin_profile_id: uuid::Uuid) {
    let team_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name)
         VALUES ('temper-system', 'Temper System')
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("ensure temper-system gating team");

    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         VALUES ($1, $2, 'owner')
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team_id)
    .bind(admin_profile_id)
    .execute(pool)
    .await
    .expect("add admin to temper-system team");

    sqlx::query(
        "UPDATE kb_system_settings SET gating_team_slug = 'temper-system', updated = now()",
    )
    .execute(pool)
    .await
    .expect("enable invite_only mode");

    // D11: the admin keeps access + admin-ness through the mode flip via standing + governance, not
    // the gating-team ownership written above (which no longer confers either).
    approved_admin(pool, admin_profile_id).await;
}

/// Configure the gating team and make `profile_id` its OWNER — i.e. a system admin.
///
/// Deliberately does NOT flip `access_mode`: production runs `'open'`, and the machine-client
/// authorization check is load-bearing precisely because the router's `require_system_access`
/// gate admits everyone under `'open'`. Testing under `'open'` is testing what prod does.
/// (Contrast [`enable_invite_only`], which also flips the mode.)
pub async fn make_system_admin(pool: &PgPool, profile_id: uuid::Uuid) {
    add_to_gating_team(pool, profile_id, "owner").await;
    // Under D11 gating-team ownership no longer confers admin-ness; the governance grant + approved
    // standing do. The gating-team membership above is retained for topology parity only.
    approved_admin(pool, profile_id).await;
}

/// Ensure the `temper-system` gating team exists, is configured as the gating team, and holds
/// `profile_id` at `role`. Roles other than `owner` do NOT confer system-adminhood —
/// `is_system_admin` requires `owner` — which is what the D4a escalation test turns on.
///
/// `temper-system` already EXISTS in a migrated database (the L0 kernel migration creates it),
/// and the auto-join generalization means a freshly provisioned profile may ALREADY hold a
/// `watcher` row on it. Both writes are therefore upserts: a plain INSERT would conflict, and a
/// `DO NOTHING` would silently leave the profile a watcher.
pub async fn add_to_gating_team(pool: &PgPool, profile_id: uuid::Uuid, role: &str) {
    let team_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name)
         VALUES ('temper-system', 'Temper System')
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("ensure temper-system gating team");

    sqlx::query(
        "UPDATE kb_system_settings SET gating_team_slug = 'temper-system', updated = now()",
    )
    .execute(pool)
    .await
    .expect("configure gating team slug");

    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         VALUES ($1, $2, $3::text::team_role)
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team_id)
    .bind(profile_id)
    .bind(role)
    .execute(pool)
    .await
    .expect("add profile to gating team");
}

/// Grant a profile explicit `can_write` on a cognitive map — the post-Q-A authoring capability
/// (`cogmap_authorable_by_profile` = an explicit `kb_access_grants` write row; root-team membership
/// confers READ, not write). Needed where a principal must AUTHOR a map it can otherwise only read —
/// e.g. opening a self-attributed invocation, which the F2 write-gate now requires. `granted_by` is the
/// grantee itself (an e2e bootstrap standing in for a real delegated grant).
pub async fn grant_cogmap_write(pool: &PgPool, cogmap: uuid::Uuid, profile: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("grant cogmap write");
}

/// Grant `profile_id` an `approved` `kb_principal_standing` — the D11 front door
/// (`has_system_access`). A fresh principal is born `Denied`, so any second/third-user test whose
/// actor must act on a gated route approves it with this first.
pub async fn approve(pool: &PgPool, profile_id: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         VALUES ($1, 'approved')
         ON CONFLICT (profile_id) DO UPDATE SET state = 'approved', updated = now()",
    )
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("approve standing");
}

/// Approve then revoke `subject`, leaving standing `revoked` — a fixture for the D15 paths
/// (re-request refusal, review markers). It moves through `approved` first so the illegal
/// `denied -> revoked` transition is never simulated. Stays direct-SQL even though Task 13's admin
/// `revoke` endpoint now exists: this fixture has no admin *token* to call it with (the app
/// principal holds standing but not governance), so `_admin` remains the intended-but-unused actor.
pub async fn approve_then_revoke(app: &E2eTestApp, _admin: uuid::Uuid, subject: uuid::Uuid) {
    approve(&app.pool, subject).await;
    sqlx::query(
        "UPDATE kb_principal_standing SET state = 'revoked', updated = now() WHERE profile_id = $1",
    )
    .bind(subject)
    .execute(&app.pool)
    .await
    .expect("revoke standing");
}

/// Configure `temper-system` as the gating team WITHOUT touching `access_mode`. A join request has
/// to attach to the gating team (spec D9: `create_join_request` resolves `gating_team_slug` and
/// errors if none is set), so a request test needs one configured even while the instance is
/// nominally `open` — which is exactly the interim state that proves the *mode* no longer gates.
pub async fn configure_gating_team(pool: &PgPool) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
        .execute(pool)
        .await
        .expect("configure gating team");
}

/// Provision the standard second user (`generate_second_user_jwt`) via the real auth path and grant
/// it `approved` standing, so it clears the front door and a test exercises the ENDPOINT authz
/// (visibility, ownership) rather than the system-access gate. Returns its profile id.
pub async fn provision_and_approve_second(app: &E2eTestApp) -> uuid::Uuid {
    let token = generate_second_user_jwt();
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("provision second user");
    let body: serde_json::Value = resp.json().await.expect("second-user profile json");
    let id: uuid::Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    approve(&app.pool, id).await;
    id
}

/// Make `profile_id` a system admin under D11: an `approved` standing (front door) plus a
/// `kb_principal_governance` grant (`is_system_admin`). Gating-team ownership confers neither now.
pub async fn approved_admin(pool: &PgPool, profile_id: uuid::Uuid) {
    approve(pool, profile_id).await;
    sqlx::query(
        "INSERT INTO kb_principal_governance (profile_id) VALUES ($1)
         ON CONFLICT (profile_id) DO NOTHING",
    )
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("grant governance");
}

/// Restore, under D11, the ambient access open-mode used to confer on the app principal centrally.
///
/// The principal is now born `Denied` on its first authenticated request, so a gated route would
/// 403. Two steps: (1) a warm-up request drives the REAL auth path, which JIT-provisions the profile
/// in `require_auth` middleware — correct handle, per-surface emitters, default context, and a
/// genuinely JIT-created auth link (behaviors the provisioning tests assert), regardless of the
/// response status; (2) grant it an `approved` standing so every subsequent gated request is
/// admitted. Deliberately NOT governance: under open mode the app principal held the front door but
/// was not a system admin (the gating slug was empty), and admin-deny tests depend on that.
pub async fn approve_app_principal(addr: std::net::SocketAddr, token: &str, pool: &PgPool) {
    // `/api/profile` is on the auth-only router; `require_auth` provisions before any gate.
    let _ = reqwest::Client::new()
        .get(format!("http://{addr}/api/profile"))
        .bearer_auth(token)
        .send()
        .await;
    // The app principal's email is constant across every setup variant.
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'approved' FROM kb_profiles WHERE email = 'e2e@test.example.com'
         ON CONFLICT (profile_id) DO UPDATE SET state = 'approved', updated = now()",
    )
    .execute(pool)
    .await
    .expect("approve app principal standing");
}

/// No-op: `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` already provisions
/// an isolated database with `migrations/` (including the canonical system seed:
/// the `handle='system'` actor, `kb_system_settings(access_mode='open')`, the
/// event-type registry, and the global lenses). There is no shared state to
/// scrub. The e2e principal (`e2e-test-user`) is auto-provisioned on its first
/// authenticated request (profile + per-surface emitter entities + a default
/// context). Tests that need named contexts create them through the API.
///
/// Retained so existing call sites keep compiling. The legacy body scrubbed a
/// shared DB and seeded a fixed-UUID System resource against tables/columns the
/// substrate retired (`kb_resource_audits`, `kb_doc_type_id`,
/// `kb_device_sync_state`, the `0004-`/`0099-` seed identities).
async fn clean_and_seed(_pool: &PgPool) {}

/// Build an `E2eTestApp` from a pool provided by `#[sqlx::test]`.
pub async fn setup(pool: PgPool) -> E2eTestApp {
    setup_with_recorder(pool, None).await
}

/// [`setup`], with the MCP relay-trust side of the network door LIVE: the API config
/// carries [`TEST_MCP_SERVICE_SECRET`], so the relay-trust middleware validates the
/// service credential and honors the `mcp` attribution carrier beside it. The
/// resources-family suites drive their tools through this app's real listener — the
/// same requests the API would receive from the deployed relay. Requests that carry
/// neither credential nor carrier behave exactly as under plain [`setup`], so only
/// the suites that cross the door need this variant.
pub async fn setup_relay(pool: PgPool) -> E2eTestApp {
    setup_with_recorder_and_blob(pool, None, None, Some(TEST_MCP_SERVICE_SECRET)).await
}

/// Every request path the test server received, in order.
///
/// A **wire-level** record: the CLI runs as a real subprocess against a real socket, so this is
/// what actually crossed it, not what the client library believes it sent. That distinction is
/// the whole point — a test asserting on the CLI's own bookkeeping cannot witness a claim about
/// what was transferred.
pub type RequestLog = Arc<Mutex<Vec<String>>>;

/// [`setup`], plus a record of every path the server is asked for.
///
/// Reach for this when the property under test is *which route was called* — most sharply, when a
/// route must **not** be. Asserting that some endpoint was never touched is not expressible any
/// other way: the absence of a request leaves no trace in a response, a database, or a process's
/// output.
pub async fn setup_recording(pool: PgPool) -> (E2eTestApp, RequestLog) {
    let log: RequestLog = Arc::new(Mutex::new(Vec::new()));
    let app = setup_with_recorder(pool, Some(log.clone())).await;
    (app, log)
}

async fn record_request_path(
    axum::extract::State(log): axum::extract::State<RequestLog>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    log.lock()
        .expect("request log is not poisoned")
        .push(req.uri().path().to_string());
    next.run(req).await
}

async fn setup_with_recorder(pool: PgPool, recorder: Option<RequestLog>) -> E2eTestApp {
    setup_with_recorder_and_blob(pool, recorder, None, None).await
}

/// [`setup`], with the blob flow LIVE: a `BlobConfig` on the API config (the D9
/// policy vocabularies at their defaults) and an [`InMemoryBlobStore`] standing in for
/// the Vercel Blob provider. `AppState::new` hardwires the real provider client from the
/// config, so the store is swapped in afterwards — the pub field exists for exactly this
/// seam. Every byte the tests commit lands in the in-process map; nothing leaves the box.
pub async fn setup_with_blob_store(pool: PgPool) -> E2eTestApp {
    let blob_config = temper_services::config::BlobConfig {
        store_id: "test-blob-store".to_string(),
        read_write_token: None,
        credential_mode: temper_services::config::BlobCredentialMode::Token,
        oidc_token_source: std::sync::Arc::new(|| None),
        max_bytes: 100 * 1024 * 1024,
        allowlist: vec![
            "image/png".into(),
            "image/jpeg".into(),
            "image/webp".into(),
            "image/svg+xml".into(),
            "image/gif".into(),
            "application/pdf".into(),
        ],
        single_request_max_bytes: 4 * 1024 * 1024,
    };
    let store: std::sync::Arc<dyn temper_substrate::blob_store::BlobStore> =
        std::sync::Arc::new(temper_substrate::blob_store::InMemoryBlobStore::default());
    setup_with_recorder_and_blob(pool, None, Some((blob_config, store)), None).await
}

/// Like [setup_with_blob_store], but the caller SUPPLIES the store — the parity
/// suite's shared-store fixture: the app's blob routes and the test's store handle
/// must be the SAME bytes for a seeded row to be readable through the door. The
/// caller's `single_request_max_bytes` rides the app's own config (the door's
/// threshold face reads it there).
pub async fn setup_with_blob_store_shared(
    pool: PgPool,
    store: std::sync::Arc<temper_substrate::blob_store::InMemoryBlobStore>,
    single_request_max_bytes: usize,
) -> E2eTestApp {
    let blob_config = temper_services::config::BlobConfig {
        store_id: "test-blob-store".to_string(),
        read_write_token: None,
        credential_mode: temper_services::config::BlobCredentialMode::Token,
        oidc_token_source: std::sync::Arc::new(|| None),
        max_bytes: 100 * 1024 * 1024,
        allowlist: vec![
            "image/png".into(),
            "image/jpeg".into(),
            "image/webp".into(),
            "image/svg+xml".into(),
            "image/gif".into(),
            "application/pdf".into(),
        ],
        single_request_max_bytes,
    };
    setup_with_recorder_and_blob(pool, None, Some((blob_config, store)), None).await
}

/// Like [setup_with_blob_store], but with the caller's own single-request ceiling —
/// a test that pins the threshold refusal needs the number in ITS fixture, not the
/// operator default (the door's commit-threshold refusal names the app's config).
pub async fn setup_with_blob_store_with_ceiling(
    pool: PgPool,
    single_request_max_bytes: usize,
) -> E2eTestApp {
    let mut blob_config = temper_services::config::BlobConfig {
        store_id: "test-blob-store".to_string(),
        read_write_token: None,
        credential_mode: temper_services::config::BlobCredentialMode::Token,
        oidc_token_source: std::sync::Arc::new(|| None),
        max_bytes: 100 * 1024 * 1024,
        allowlist: vec![
            "image/png".into(),
            "image/jpeg".into(),
            "image/webp".into(),
            "image/svg+xml".into(),
            "image/gif".into(),
            "application/pdf".into(),
        ],
        single_request_max_bytes: 4 * 1024 * 1024,
    };
    blob_config.single_request_max_bytes = single_request_max_bytes;
    let store: std::sync::Arc<dyn temper_substrate::blob_store::BlobStore> =
        std::sync::Arc::new(temper_substrate::blob_store::InMemoryBlobStore::default());
    setup_with_recorder_and_blob(pool, None, Some((blob_config, store)), None).await
}

async fn setup_with_recorder_and_blob(
    pool: PgPool,
    recorder: Option<RequestLog>,
    blob: Option<(
        temper_services::config::BlobConfig,
        std::sync::Arc<dyn temper_substrate::blob_store::BlobStore>,
    )>,
    relay_secret: Option<&str>,
) -> E2eTestApp {
    clean_and_seed(&pool).await;

    // --- Server setup ---
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.pub"))
            .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);

    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: TEST_AUDIENCE.to_string(),
            mcp_audience: TEST_AUDIENCE.to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        cron_host: None,
        sensitivity_sweep_salt: None,
        sensitivity_sweep_enabled: false,
        mcp_service_secret: relay_secret.map(str::to_string),
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
        rate_limit: None,
        blob: blob.as_ref().map(|(c, _)| c.clone()),
        blob_disabled_by_policy: false,
    };

    let mut state = AppState::new(pool.clone(), jwks_store, api_config);
    // `AppState::new` builds the real provider client from the config; the e2e seam is the
    // swap to the caller's store (the in-memory fake when `blob` is Some).
    if let Some((_, store)) = blob {
        state.blob_store = Some(store);
    }
    let app = create_app(state);
    let app = match recorder {
        Some(log) => app.layer(axum::middleware::from_fn_with_state(
            log,
            record_request_path,
        )),
        None => app,
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener.local_addr().expect("Failed to get local addr");

    let serve = ServeGuard(tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Test server failed");
    }));

    // --- Config + client setup (no disk reads) ---
    let token = generate_test_jwt("e2e-test-user", "e2e@test.example.com");

    // D11: the app principal is born Denied on first auth. Provision it via the real path and grant
    // approved standing so the tests' gated requests are admitted (open-mode's ambient, restored).
    approve_app_principal(addr, &token, &pool).await;

    let vault_dir = TempDir::new().expect("Failed to create temp vault");
    std::fs::create_dir_all(vault_dir.path().join(".temper"))
        .expect("Failed to create .temper dir");

    let temper_config = TemperConfig {
        vault: CloudVaultConfig {
            path: vault_dir.path().to_str().unwrap().to_string(),
        },
        cloud: CloudSection {
            api_url: format!("http://{addr}"),
        },
        ..TemperConfig::default()
    };

    let stored_auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: token.clone().into(),
        refresh_token: None,
        expires_at: Utc::now() + Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-test-device".to_string()),
    };

    let store: std::sync::Arc<dyn temper_client::auth::TokenStore> =
        std::sync::Arc::new(MemoryTokenStore::with_auth(stored_auth));
    // The harness client stands in for the CLI in cloud mode, so it declares `Surface::CliCloud`
    // and sends `X-Temper-Surface: cli` on every request.
    let client = temper_client::config::build_client_from(
        &temper_config,
        store,
        temper_workflow::operations::Surface::CliCloud,
    )
    .expect("Failed to build test client");

    let cli_config = temper_cli::config::load_from(&temper_config, None);

    E2eTestApp {
        addr,
        pool,
        client,
        reqwest_client: reqwest::Client::new(),
        config: temper_config,
        cli_config,
        token,
        vault_dir,
        _serve: serve,
    }
}

/// Build an `E2eTestApp` from a pool provided by `#[sqlx::test]`, keyed with
/// the EdDSA test fixture instead of RSA. Identical to `setup` in every other
/// respect (same `auth_issuer`/`auth_audience`, same auto-provisioned token
/// user) so the two harnesses only differ in signing algorithm.
pub async fn setup_eddsa(pool: PgPool) -> E2eTestApp {
    setup_eddsa_with_provider(pool, "test-provider").await
}

/// Like [`setup_eddsa`] but with a caller-chosen `auth_provider_name`, so a test can assert
/// provider namespacing (e.g. `saml:test-idp`) on the JIT-created `kb_profile_auth_links` row.
pub async fn setup_eddsa_with_provider(pool: PgPool, provider: &str) -> E2eTestApp {
    clean_and_seed(&pool).await;

    // --- Server setup ---
    let decoding_key =
        jsonwebtoken::DecodingKey::from_ed_pem(include_bytes!("../fixtures/test_ed25519.pub.pem"))
            .expect("Failed to load test Ed25519 public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::EdDSA);

    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: TEST_AUDIENCE.to_string(),
            mcp_audience: TEST_AUDIENCE.to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: provider.to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        cron_host: None,
        sensitivity_sweep_salt: None,
        sensitivity_sweep_enabled: false,
        mcp_service_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
        rate_limit: None,
        blob: None,
        blob_disabled_by_policy: false,
    };

    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener.local_addr().expect("Failed to get local addr");

    let serve = ServeGuard(tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Test server failed");
    }));

    // --- Config + client setup (no disk reads) ---
    let token = generate_test_jwt_eddsa("e2e-test-user", "e2e@test.example.com");

    // D11: provision + approve the app principal via the real path (see `approve_app_principal`).
    approve_app_principal(addr, &token, &pool).await;

    let vault_dir = TempDir::new().expect("Failed to create temp vault");
    std::fs::create_dir_all(vault_dir.path().join(".temper"))
        .expect("Failed to create .temper dir");

    let temper_config = TemperConfig {
        vault: CloudVaultConfig {
            path: vault_dir.path().to_str().unwrap().to_string(),
        },
        cloud: CloudSection {
            api_url: format!("http://{addr}"),
        },
        ..TemperConfig::default()
    };

    let stored_auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: token.clone().into(),
        refresh_token: None,
        expires_at: Utc::now() + Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-test-device".to_string()),
    };

    let store: std::sync::Arc<dyn temper_client::auth::TokenStore> =
        std::sync::Arc::new(MemoryTokenStore::with_auth(stored_auth));
    // The harness client stands in for the CLI in cloud mode, so it declares `Surface::CliCloud`
    // and sends `X-Temper-Surface: cli` on every request.
    let client = temper_client::config::build_client_from(
        &temper_config,
        store,
        temper_workflow::operations::Surface::CliCloud,
    )
    .expect("Failed to build test client");

    let cli_config = temper_cli::config::load_from(&temper_config, None);

    E2eTestApp {
        addr,
        pool,
        client,
        reqwest_client: reqwest::Client::new(),
        config: temper_config,
        cli_config,
        token,
        vault_dir,
        _serve: serve,
    }
}

/// Chunks for `body` as the REAL chunker produces them, packed with an inert constant vector —
/// ONNX-free, and by construction a fresh chunking: the write path applies the blocking policy,
/// so a client chunk set whose hashes no fresh chunking reproduces is a chunker drift the op
/// refuses. Every e2e fixture that ingests a body with explicit chunks must build them here.
pub fn chunked(text: &str, fill: f32) -> Vec<temper_core::types::ingest::PackedChunk> {
    temper_ingest::chunk::chunk_markdown(text)
        .into_iter()
        .map(|c| temper_core::types::ingest::PackedChunk {
            chunk_index: c.chunk_index,
            header_path: c.header_path,
            heading_depth: c.heading_depth,
            content: c.content,
            content_hash: c.content_hash,
            embedding: vec![fill; 768],
            embedded_with: None,
        })
        .collect()
}

/// One refusal face of the MCP `context_anchor` resolver (`@me/<slug>`, `@<handle>/<slug>`,
/// `+<team>/<slug>`, or a UUID → the context id), as a caller of a context-addressed tool meets
/// it: the ref, the identity that sends it, and the byte-exact `invalid_params` sentence it
/// answers.
pub struct AnchorFace {
    pub label: &'static str,
    pub context_ref: String,
    pub parts: axum::http::request::Parts,
    pub expected: String,
}

/// Every refusal face of `context_anchor`, constructed against this app, for the two tools that
/// address a context by ref (the context orientation family in `cognitive_maps.rs` and
/// `resource_reblock`'s `scope=context`). Both suites pin the SAME table through their own tool,
/// so the two anchors cannot drift apart.
///
/// Built once, pinned green against the in-process resolver, then carried through the relay to
/// `GET /api/contexts/resolve` (the network door's teardown). The faces, per resolver arm:
///
/// - a malformed ref refuses at the local parse, with the shared grammar's sentence;
/// - the `@me` arm's miss names the caller's own slug;
/// - **no existence oracle** on the UUID and `@<handle>` arms: a stranger's view of the owner's
///   private context, an id naming nothing, the owner's real ref, an absent slug and an unknown
///   handle all answer one sentence;
/// - the `+<team>` arm: an absent team names the team; an existing team the caller is not in
///   answers the resolver's existing `Forbidden` (which discloses the team exists — documented at
///   the resolver, kept, not introduced here); a member's miss names the slug.
///
/// Every parts value carries the claims extension beside the bearer, as the JWT middleware
/// injects them, so the same table drives the in-process resolver (which reads the claims) and
/// the relay (which forwards the bearer).
pub async fn context_anchor_faces(app: &E2eTestApp) -> Vec<AnchorFace> {
    use temper_core::context_ref::ContextOwnerRef;
    use temper_core::types::team::TeamCreateRequest;

    fn parts_for(token: &str, sub: &str, email: Option<&str>) -> axum::http::request::Parts {
        axum::http::Request::builder()
            .extension(temper_mcp_server::BearerToken(token.to_string()))
            .extension(temper_services::auth::RawJwtClaims {
                sub: sub.to_string(),
                email: email.map(str::to_string),
                email_verified: None,
                azp: None,
                gty: None,
                exp: (Utc::now() + Duration::hours(1)).timestamp(),
                iat: 0,
            })
            .body(())
            .expect("anchor-face parts build")
            .into_parts()
            .0
    }

    app.client.profile().get().await.expect("owner profile");
    provision_and_approve_second(app).await;
    let owner = || app.direct_parts();
    let stranger_token = generate_second_user_jwt();
    let stranger = || {
        parts_for(
            &stranger_token,
            "e2e-second-user",
            Some("second@test.example.com"),
        )
    };

    let private = app
        .client
        .contexts()
        .create("anchor faces private", None)
        .await
        .expect("create the owner's private context");
    app.client
        .teams()
        .create(&TeamCreateRequest {
            slug: "anchor-faces-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("create the owner's team");
    let team_ctx = app
        .client
        .contexts()
        .create(
            "anchor faces team home",
            Some(ContextOwnerRef::Team("anchor-faces-team".to_owned())),
        )
        .await
        .expect("create the team's context");

    const UNREADABLE: &str = "context not found: context not found or not readable";
    let face = |label, context_ref: String, parts, expected: &str| AnchorFace {
        label,
        context_ref,
        parts,
        expected: expected.to_owned(),
    };
    vec![
        face(
            "malformed: a bare name",
            "not a ref".to_owned(),
            owner(),
            "invalid context ref: not a context ref: bare names are not addressable — use a UUID \
             or `@owner/slug` (got \"not a ref\")",
        ),
        face(
            "malformed: owner without a slug",
            "@me".to_owned(),
            owner(),
            "invalid context ref: context ref is missing the `/slug` after the owner (got \"@me\")",
        ),
        face(
            "@me: the caller's own miss names the slug",
            "@me/no-such-slug".to_owned(),
            owner(),
            "context not found: context no-such-slug not found or not readable",
        ),
        face(
            "UUID: another principal's private context",
            private.id.to_string(),
            stranger(),
            UNREADABLE,
        ),
        face(
            "UUID: an id naming nothing",
            uuid::Uuid::now_v7().to_string(),
            stranger(),
            UNREADABLE,
        ),
        face(
            "@handle: another principal's real ref",
            format!("{}/{}", private.owner_ref, private.slug),
            stranger(),
            UNREADABLE,
        ),
        face(
            "@handle: an absent slug",
            format!("{}/no-such-context", private.owner_ref),
            stranger(),
            UNREADABLE,
        ),
        face(
            "@handle: an unknown handle",
            "@no-such-handle/no-such-context".to_owned(),
            stranger(),
            UNREADABLE,
        ),
        face(
            "+team: an absent team",
            "+no-such-team/no-such-context".to_owned(),
            stranger(),
            "context not found: team no-such-team not found or not readable",
        ),
        face(
            "+team: an existing team, caller not a member",
            format!("+anchor-faces-team/{}", team_ctx.slug),
            stranger(),
            "context not found: Forbidden",
        ),
        face(
            "+team: a member's miss names the slug",
            "+anchor-faces-team/no-such-context".to_owned(),
            owner(),
            "context not found: context no-such-context not found or not readable",
        ),
    ]
}

/// One MCP act through the network door as the holder of `token`: `context_manage`'s
/// `create`, a relayed write — so an admission is witnessed by the profile the API
/// resolved for the bearer (the created context's `owner_ref`), and a refusal by the
/// post-edge mapping of the API's own 401/403. The auth-seam suites' MCP leg since the
/// network door's teardown removed the in-process gate they used to call.
pub async fn mcp_act_as(
    app: &E2eTestApp,
    token: &str,
) -> Result<serde_json::Value, rmcp::ErrorData> {
    let svc = app.mcp_relay_service().await;
    let res = temper_mcp::tools::contexts::context_manage(
        &svc,
        &app.relay_parts_for(token),
        serde_json::from_value(serde_json::json!({
            "action": "create",
            "name": format!("auth seam {}", uuid::Uuid::now_v7()),
        }))
        .expect("context_manage input deserializes"),
    )
    .await?;
    let text = res.content[0].as_text().expect("a text part").text.clone();
    Ok(serde_json::from_str(&text).expect("the created context row"))
}

/// The profile that owns the context `mcp_act_as` created — the identity the API resolved for
/// the act's bearer, read back from the row itself rather than from a rendered handle.
pub async fn created_context_owner(pool: &PgPool, created: &serde_json::Value) -> uuid::Uuid {
    let context_id: uuid::Uuid = created["id"]
        .as_str()
        .expect("created context id")
        .parse()
        .expect("context id parse");
    sqlx::query_scalar("SELECT owner_id FROM kb_contexts WHERE id = $1")
        .bind(context_id)
        .fetch_one(pool)
        .await
        .expect("the created context's owner")
}
