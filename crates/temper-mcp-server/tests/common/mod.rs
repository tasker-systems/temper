// Each integration test file compiles this module into its own binary, so a fixture used by one
// suite is dead code in the others. That is inherent to `tests/common`, not a sign of an unused
// helper.
#![allow(dead_code)]

//! Fixtures for the router-level integration tests.
//!
//! Every suite here assembles a real `build_router` and drives requests through it. None of them
//! reaches a tool body — and none needs a database: the router holds no pool to fake.

use jsonwebtoken::{Algorithm, DecodingKey};
use temper_auth::config::{AuthConfig, AuthMode};
use temper_mcp::BlobDoor;
use temper_mcp_server::config::{deployed_relay, DeployedRelay};
use temper_mcp_server::discovery_config::OAuthStaticConfig;
use temper_mcp_server::{DiscoveryConfig, McpServerConfig};
use temper_services::state::JwksKeyStore;

/// What the router is assembled from besides its `DiscoveryConfig`: the boot config and the key
/// store.
pub struct Edge {
    pub config: McpServerConfig,
    pub jwks_store: JwksKeyStore,
}

/// The real `build_router`, from an [`Edge`] fixture.
pub fn build_router(edge: Edge, discovery: DiscoveryConfig) -> axum::Router {
    temper_mcp_server::build_router(edge.config, edge.jwks_store, discovery)
}

/// No relay: none of these suites reaches a tool body, so the tool door is dark.
fn unconfigured_relay() -> DeployedRelay {
    deployed_relay(&|_: &str| None)
}

/// A blob-less deployment's door: closed, in the unconfigured vocabulary.
fn closed_blob_door() -> BlobDoor {
    temper_mcp_server::config::blob_door(None, false)
}

/// An edge whose key store is pre-loaded, so the auth gate actually *validates* a token
/// instead of failing to fetch a key.
///
/// This distinction has teeth. With the unreachable JWKS URL that [`state_with_cors_origins`]
/// uses, a request carrying a malformed bearer is refused with `503` — the store cannot fetch a
/// key, so the gate fails closed before it can judge the token. That is the correct direction to
/// fail, but it means a `401` assertion against that fixture would be testing the network, not the
/// gate. A static key makes the refusal a real validation verdict.
pub fn state_with_static_jwt_key() -> Edge {
    let mut edge = state_with_cors_origins(vec![]);
    edge.jwks_store =
        JwksKeyStore::with_static_key(DecodingKey::from_secret(b"witness"), Algorithm::HS256);
    edge
}

/// An edge carrying `cors_origins` and nothing else that matters to a router test.
pub fn state_with_cors_origins(cors_origins: Vec<String>) -> Edge {
    Edge {
        config: McpServerConfig {
            auth: AuthConfig {
                issuer: "unused".to_string(),
                jwks_url: "unused".to_string(),
                audience: "unused".to_string(),
                mcp_audience: "unused".to_string(),
                mode: AuthMode::ExternalIdp,
            },
            cors_origins,
            blob_door: closed_blob_door(),
            relay: unconfigured_relay(),
        },
        jwks_store: JwksKeyStore::new("https://example.invalid/.well-known/jwks.json".to_string()),
    }
}

/// An edge whose auth identity names DISTINCT API and MCP audiences, with the same static
/// HS256 key as [`state_with_static_jwt_key`] — the dedicated-MCP-resource instance shape. The
/// audience-set tests pin the gate's contract against it: a token for either audience passes,
/// a token for neither does not.
pub fn state_with_distinct_audiences() -> Edge {
    Edge {
        config: McpServerConfig {
            auth: AuthConfig {
                issuer: "https://as.test".to_string(),
                jwks_url: "unused".to_string(),
                audience: "https://inst.test/api".to_string(),
                mcp_audience: "https://inst.test/mcp".to_string(),
                mode: AuthMode::ExternalIdp,
            },
            cors_origins: vec![],
            blob_door: closed_blob_door(),
            relay: unconfigured_relay(),
        },
        jwks_store: JwksKeyStore::with_static_key(
            DecodingKey::from_secret(b"witness"),
            Algorithm::HS256,
        ),
    }
}

/// A `DiscoveryConfig` with **no** `mcp_client_id`, which is what makes `/oauth/register` answer
/// `503 SERVICE_UNAVAILABLE` from inside the handler rather than failing earlier.
pub fn discovery_config() -> DiscoveryConfig {
    DiscoveryConfig {
        mcp_base_url: "https://temper.invalid".to_string(),
        mcp_client_id: None,
        oauth: OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: false,
        },
    }
}
