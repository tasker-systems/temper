use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use temper_api::config::ApiConfig;
use temper_api::routes::create_app;
use temper_api::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ApiConfig::from_env().expect("Failed to load config from environment");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let jwks_store = JwksKeyStore::new(config.jwks_url.clone());
    let port = config.port;
    let backend_selection = temper_api::services::backend_selection_service::read(&pool)
        .await
        .expect("Failed to read backend selection flag");
    let state = AppState::new(pool, jwks_store, config).with_backend_selection(backend_selection);
    let app = create_app(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.expect("Failed to bind");
    tracing::info!("temper-api listening on {addr}");

    axum::serve(listener, app).await.expect("Server failed");
}
