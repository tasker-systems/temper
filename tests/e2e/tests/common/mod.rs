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

use temper_api::{
    config::ApiConfig,
    create_app,
    state::{AppState, JwksKeyStore},
};
use temper_client::auth::{MemoryTokenStore, Provider, StoredAuth};
use temper_core::types::config::{CloudSection, CloudVaultConfig, TemperConfig};

// Well-known UUIDs from seed migration.
pub const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
pub const TEMPER_CONTEXT_ID: &str = "00000000-0000-0000-0003-000000000001";
pub const RESEARCH_DOC_TYPE_ID: &str = "00000000-0000-0000-0001-000000000004";
pub const TEMPER_SYSTEM_TEAM_ID: &str = "00000000-0000-0000-0000-000000000002";
pub const TEMPER_SYSTEM_GENERAL_CONTEXT_ID: &str = "00000000-0000-0000-0000-000000000003";

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

/// Enable invite-only mode in tests by adding admin to temper-system team and flipping the setting.
pub async fn enable_invite_only(pool: &PgPool, admin_profile_id: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at)
         VALUES (gen_random_uuid(), $1::uuid, $2, 'owner', now())
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(uuid::Uuid::parse_str(TEMPER_SYSTEM_TEAM_ID).unwrap())
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

/// Seed fixtures: delete test data, insert stable seed resource.
async fn clean_and_seed(pool: &PgPool) {
    sqlx::query("DELETE FROM kb_resource_audits")
        .execute(pool)
        .await
        .expect("clean kb_resource_audits");

    sqlx::query(
        "DELETE FROM kb_events WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean kb_events");

    sqlx::query("DELETE FROM kb_device_sync_state")
        .execute(pool)
        .await
        .expect("clean kb_device_sync_state");
    sqlx::query("DELETE FROM kb_transfers")
        .execute(pool)
        .await
        .expect("clean kb_transfers");
    // Reset system settings to open mode (before team cleanup)
    sqlx::query("UPDATE kb_system_settings SET access_mode = 'open', gating_team_slug = NULL, updated = now()")
        .execute(pool)
        .await
        .expect("reset kb_system_settings");

    sqlx::query("DELETE FROM kb_join_requests")
        .execute(pool)
        .await
        .expect("clean kb_join_requests");

    sqlx::query("DELETE FROM kb_team_invitations")
        .execute(pool)
        .await
        .expect("clean kb_team_invitations");
    sqlx::query("DELETE FROM kb_team_resources")
        .execute(pool)
        .await
        .expect("clean kb_team_resources");
    sqlx::query(
        "DELETE FROM kb_team_members WHERE team_id != '00000000-0000-0000-0000-000000000002'::uuid",
    )
    .execute(pool)
    .await
    .expect("clean kb_team_members");
    sqlx::query("DELETE FROM kb_teams WHERE id != '00000000-0000-0000-0000-000000000002'::uuid")
        .execute(pool)
        .await
        .expect("clean kb_teams");

    sqlx::query(
        "DELETE FROM kb_resources WHERE owner_profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test resources");

    sqlx::query(
        "DELETE FROM kb_profile_auth_links WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test auth links");

    sqlx::query(
        "DELETE FROM kb_profiles WHERE id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test profiles");

    sqlx::query(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES (
            '00000000-0000-0000-0099-000000000001',
            $1, $2,
            'test://seed-resource',
            'Seed Research Doc',
            'seed-research-doc',
            $3, $3,
            true, now(), now()
        )
        ON CONFLICT (id) DO UPDATE SET updated = now()
        "#,
    )
    .bind(uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap())
    .bind(uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap())
    .bind(uuid::Uuid::parse_str(SYSTEM_PROFILE_ID).unwrap())
    .execute(pool)
    .await
    .expect("seed resource");
}

/// Build an `E2eTestApp` from a pool provided by `#[sqlx::test]`.
pub async fn setup(pool: PgPool) -> E2eTestApp {
    clean_and_seed(&pool).await;

    // --- Server setup ---
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.pub"))
            .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key);

    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
    };

    let backend_selection = temper_api::services::backend_selection_service::read(&pool)
        .await
        .expect("read backend selection flag");
    let state = AppState::new(pool.clone(), jwks_store, api_config)
        .with_backend_selection(backend_selection);
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
    let client = temper_client::config::build_client_from(&temper_config, store)
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
