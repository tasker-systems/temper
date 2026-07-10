#![allow(dead_code)]
//! Shared e2e test infrastructure.
//!
//! `E2eTestApp` starts an in-process Axum server backed by an isolated
//! per-test database and builds a `TemperClient` with injected config
//! (no disk reads, no env var manipulation).

pub mod tracing_layer;

use std::net::SocketAddr;

use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::net::TcpListener;

use temper_api::create_app;
use temper_client::auth::{MemoryTokenStore, Provider, StoredAuth};
use temper_core::types::config::{CloudSection, CloudVaultConfig, TemperConfig};
use temper_services::{
    config::ApiConfig,
    state::{AppState, JwksKeyStore},
};

/// A running e2e test environment with in-process API server and injected client.
pub struct E2eTestApp {
    pub addr: SocketAddr,
    pub pool: PgPool,
    pub client: temper_client::TemperClient,
    pub reqwest_client: reqwest::Client,
    pub config: TemperConfig,
    pub cli_config: temper_cli::config::Config,
    pub token: String,
    pub vault_dir: TempDir,
}

impl E2eTestApp {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
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
    let bin = temper_bin_path();
    let url = app.base_url();
    let token = app.token.clone();
    let args_owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();

    // Materialize the test TemperConfig to a TOML file inside the test's
    // vault temp directory so the spawned CLI can read it. The path lives
    // alongside the vault projection so it shares the test's cleanup.
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("test-temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config for CLI invocation");

    tokio::task::spawn_blocking(move || {
        std::process::Command::new(&bin)
            .env("TEMPER_API_URL", &url)
            .env("TEMPER_TOKEN", &token)
            .env("TEMPER_GLOBAL_CONFIG", &config_path)
            .args(&args_owned)
            .output()
    })
    .await
    .expect("spawn_blocking join")
}

/// JWT claims for test tokens.
#[derive(Debug, Serialize, Deserialize)]
struct TestClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: String,
    iat: i64,
    exp: i64,
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
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign test JWT")
}

/// JWT claims for a machine (`client_credentials`) test token. `gty` is the definitive
/// machine signal `normalize_machine` keys on; `azp` carries the client id. No email:
/// a machine has none.
#[derive(Debug, Serialize, Deserialize)]
struct MachineTestClaims {
    sub: String,
    azp: String,
    gty: String,
    iss: String,
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
        "UPDATE kb_system_settings SET access_mode = 'invite_only', gating_team_slug = 'temper-system', updated = now()",
    )
    .execute(pool)
    .await
    .expect("enable invite_only mode");
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
    clean_and_seed(&pool).await;

    // --- Server setup ---
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.pub"))
            .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);

    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
    };

    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener.local_addr().expect("Failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Test server failed");
    });

    // --- Config + client setup (no disk reads) ---
    let token = generate_test_jwt("e2e-test-user", "e2e@test.example.com");

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
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: provider.to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
    };

    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener.local_addr().expect("Failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Test server failed");
    });

    // --- Config + client setup (no disk reads) ---
    let token = generate_test_jwt_eddsa("e2e-test-user", "e2e@test.example.com");

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
    }
}
