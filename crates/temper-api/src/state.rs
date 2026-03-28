use sqlx::PgPool;
use std::sync::Arc;

use crate::config::ApiConfig;

/// Placeholder for JWKS key store — implemented in Task 2.
#[derive(Debug, Clone)]
pub struct JwksKeyStore;

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
