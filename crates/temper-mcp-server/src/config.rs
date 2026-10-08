//! The MCP server's boot configuration — what this deployment reads from its environment, and
//! nothing the API alone needs.
//!
//! **Not `ApiConfig`.** The API's configuration carries the database URL, the broker, Slack, rate
//! limits and every gate secret. This process serves a JWT edge and relays every tool act, so it
//! reads four things — and reads each through the SAME parser the API boots with, because the two
//! doors must agree about them:
//!
//! - the auth identity (`temper_auth::config::parse_auth_config`): a token the API accepts must
//!   verify here, and the reverse;
//! - the shared-secret floor on the secrets this process presents, `TEMPER_MCP_SERVICE_SECRET`
//!   and `TEMPER_EDGE_PROXY_SECRET` (`temper_auth::config::check_shared_secret_strength`);
//! - the CORS origins (`temper_services::cors::parse_cors_origins`);
//! - the blob posture (`temper_services::config::parse_blob`), handed to the tool layer as a plain
//!   [`BlobDoor`] with the refusal chosen by `blob_service::blob_refusal`.
//!
//! `DATABASE_URL` is never read. (On Vercel the variable is still in this function's process
//! environment — environment variables are project-scoped and the API shares the project — but
//! this process neither requires, reads nor connects with it.)
//!
//! It also states the relay, which the tool layer accepts as plain values and never reads: the
//! API's base URL (`TEMPER_API_BASE_URL`, with the [`RELAY_REQUEST_TIMEOUT`] this function's
//! budget allows) and the service credential the deployed seam presents. Either missing leaves
//! the edge up and the tool door dark, refusing every relayed act with [`DeployedRelay`]'s
//! sentence — the deployment-misconfiguration wording is this shell's, never the tool layer's.
//!
//! What the API's boot checks and this one does not: the cross-secret distinctness check and the
//! strength floor on secrets this process never holds. Distinctness is a property of the
//! VALIDATOR's environment, and the API is the only thing that validates
//! `TEMPER_MCP_SERVICE_SECRET` — so the check that matters is the API's, which refuses an
//! overlap in its own environment. A refusal here would protect no secret the API's check
//! misses, and would add a fail-closed boot condition to the agent-facing door: a boot refusal
//! fires at cold start, after a deployment is promoted, so it takes the door dark rather than
//! failing the deploy.

use std::time::Duration;
use temper_auth::config::{
    check_shared_secret_strength, parse_auth_config, shared_secret, AuthConfig, AuthConfigError,
};

use temper_mcp::{BlobDoor, RelayConfig};
use temper_services::config::BlobConfig;

use crate::seam::DeployedDoorSeam;

/// The relay's per-request client timeout.
///
/// Strictly below the 60 s `maxDuration` this function runs inside (`vercel.json`), with
/// shaping margin — a hung API call must surface as the rmcp-shaped refusal the tool layer
/// maps, never as the platform killing the function mid-flight (design §2.1, ruling 6).
/// The stock temper-client ceiling (75 s) is sized ABOVE the server's budget on purpose —
/// for a CLI that must observe what the server did — and is exactly inverted here.
pub const RELAY_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// The shared secrets this process presents on relayed acts, held to the floor.
const PRESENTED_SECRETS: [&str; 2] = ["TEMPER_MCP_SERVICE_SECRET", "TEMPER_EDGE_PROXY_SECRET"];

/// The MCP server's boot configuration.
#[derive(Clone, Debug)]
pub struct McpServerConfig {
    /// This instance's verified auth identity — the issuer, JWKS URL and audiences the JWT edge
    /// checks, and the MCP audience its protected-resource metadata advertises.
    pub auth: AuthConfig,
    /// `CORS_ORIGINS`, as the shared policy reads it.
    pub cors_origins: Vec<String>,
    /// The deployment's blob posture, as the tool layer consumes it.
    pub blob_door: BlobDoor,
    /// Where and as whom the tool layer relays, or why it cannot.
    pub relay: DeployedRelay,
}

/// The deployed relay: configured, or dark with the sentence every relayed act answers.
#[derive(Clone, Debug)]
pub enum DeployedRelay {
    /// `TEMPER_API_BASE_URL` and `TEMPER_MCP_SERVICE_SECRET` are both set and usable.
    Ready {
        config: RelayConfig,
        seam: DeployedDoorSeam,
    },
    /// The tool door is dark; health and discovery stay up. The sentence names the variable.
    Unavailable(&'static str),
}

const BASE_URL_UNSET: &str = "This MCP deployment is not configured to forward tool calls: \
     TEMPER_API_BASE_URL is unset. Health and discovery remain available; contact the operator.";
const SECRET_UNSET: &str = "This MCP deployment is not configured to forward tool calls: \
     TEMPER_MCP_SERVICE_SECRET is unset. Health and discovery remain available; contact the \
     operator.";
const SECRET_NOT_A_HEADER: &str = "TEMPER_MCP_SERVICE_SECRET is not a valid header value; \
     refusing to forward. Contact the operator.";

/// The relay this deployment's environment states. Both values are trimmed; the secret through
/// the API's own reader (`shared_secret`), so the value presented is the value the API compares.
pub fn deployed_relay(lookup: &impl Fn(&str) -> Option<String>) -> DeployedRelay {
    let Some(api_base_url) = lookup("TEMPER_API_BASE_URL")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return DeployedRelay::Unavailable(BASE_URL_UNSET);
    };
    let Some(secret) = shared_secret(lookup, "TEMPER_MCP_SERVICE_SECRET") else {
        return DeployedRelay::Unavailable(SECRET_UNSET);
    };
    let Ok(seam) = DeployedDoorSeam::new(&secret) else {
        return DeployedRelay::Unavailable(SECRET_NOT_A_HEADER);
    };
    DeployedRelay::Ready {
        config: RelayConfig::new(api_base_url, RELAY_REQUEST_TIMEOUT),
        seam: with_edge_proxy_marker(seam, lookup, &secret),
    }
}

/// Add the edge-proxy marker when `TEMPER_EDGE_PROXY_SECRET` is set. Past the boot's strength
/// floor (`PRESENTED_SECRETS`), never a refusal: the marker only exempts relayed acts from the
/// API edge's per-IP limits, so a value that cannot be sent leaves the relay working and logs
/// why. A value equal to the service secret is not
/// sent, because the marker's value is copied into firewall configuration, where the service
/// credential must never be.
fn with_edge_proxy_marker(
    seam: DeployedDoorSeam,
    lookup: &impl Fn(&str) -> Option<String>,
    service_secret: &str,
) -> DeployedDoorSeam {
    let Some(marker) = shared_secret(lookup, "TEMPER_EDGE_PROXY_SECRET") else {
        return seam;
    };
    if marker == service_secret {
        tracing::error!(
            "TEMPER_EDGE_PROXY_SECRET equals TEMPER_MCP_SERVICE_SECRET; not sending the \
             edge-proxy marker. Generate a separate value."
        );
        return seam;
    }
    match seam.clone().with_edge_proxy_secret(&marker) {
        Ok(marked) => marked,
        Err(_) => {
            tracing::error!(
                "TEMPER_EDGE_PROXY_SECRET is not a valid header value; not sending the \
                 edge-proxy marker"
            );
            seam
        }
    }
}

impl McpServerConfig {
    /// Load from the process environment. Refuses to produce a config the edge cannot serve on.
    pub fn from_env() -> Result<Self, AuthConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Load from an arbitrary lookup rather than the process environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, AuthConfigError> {
        let auth = parse_auth_config(&lookup)?;
        // Logged for the same reason the API logs it: an operator who cannot tell which mode an
        // instance is in is the operator who mis-sets these variables.
        tracing::info!(mode = %auth.mode, "auth configured");

        check_shared_secret_strength(&lookup, &PRESENTED_SECRETS)?;

        let cors_origins = temper_services::cors::parse_cors_origins(&lookup);
        let (blob, disabled_by_policy) = temper_services::config::parse_blob(&lookup);

        Ok(Self {
            auth,
            cors_origins,
            blob_door: blob_door(blob.as_ref(), disabled_by_policy),
            relay: deployed_relay(&lookup),
        })
    }
}

/// The tool layer's view of the blob posture: open with the single-request ceiling when a
/// credential resolves, else closed with the refusal the shared selector chooses — the
/// unconfigured vocabulary, or the policy vocabulary when `BLOB_ENABLED` closed the door.
pub fn blob_door(blob: Option<&BlobConfig>, disabled_by_policy: bool) -> BlobDoor {
    match blob {
        Some(c) => BlobDoor::Open {
            single_request_max_bytes: c.single_request_max_bytes,
        },
        None => BlobDoor::Closed {
            refusal: temper_services::services::blob_service::blob_refusal(disabled_by_policy)
                .to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    const AUTH: [(&str, &str); 3] = [
        ("AUTH_ISSUER", "https://tenant.auth0.com/"),
        ("JWKS_URL", "https://tenant.auth0.com/.well-known/jwks.json"),
        ("AUTH_AUDIENCE", "https://temperkb.io/api"),
    ];

    /// The boot this function makes: no `DATABASE_URL`, and it loads.
    /// FAILS IF: the server's config starts requiring the API's database URL again.
    #[test]
    fn boots_without_a_database_url() {
        let cfg = McpServerConfig::from_lookup(env(&AUTH)).expect("boots without DATABASE_URL");
        assert_eq!(cfg.auth.mcp_audience, "https://temperkb.io/api");
        assert!(cfg.cors_origins.is_empty());
        assert!(!cfg.blob_door.is_open());
    }

    /// The auth identity is the API's parser's: its refusals are this boot's refusals.
    #[test]
    fn the_auth_identity_refuses_as_the_api_does() {
        assert_eq!(
            McpServerConfig::from_lookup(env(&AUTH[..2])).unwrap_err(),
            AuthConfigError::MissingAudience
        );
    }

    /// The one secret this process presents is held to the shared floor.
    #[test]
    fn a_weak_service_secret_refuses_the_boot() {
        let mut pairs = AUTH.to_vec();
        pairs.push(("TEMPER_MCP_SERVICE_SECRET", "short"));
        assert_eq!(
            McpServerConfig::from_lookup(env(&pairs)).unwrap_err(),
            AuthConfigError::WeakSharedSecret("TEMPER_MCP_SERVICE_SECRET")
        );
    }

    /// The edge-proxy marker is presented too, so it is held to the same floor; the refusal
    /// names the variable and never its value.
    /// FAILS IF: a weak marker boots, or the refusal quotes the value.
    #[test]
    fn a_weak_edge_proxy_secret_refuses_the_boot_without_quoting_it() {
        let mut pairs = AUTH.to_vec();
        pairs.push(("TEMPER_EDGE_PROXY_SECRET", "shortmarker"));
        let err = McpServerConfig::from_lookup(env(&pairs)).unwrap_err();
        assert_eq!(
            err,
            AuthConfigError::WeakSharedSecret("TEMPER_EDGE_PROXY_SECRET")
        );
        assert!(!err.to_string().contains("shortmarker"));
    }

    /// The closed door speaks the API's vocabulary for the posture that closed it — the same
    /// sentence `AppState::blob_refusal` renders, byte for byte.
    /// FAILS IF: the server picks its own words, or picks the wrong arm for the posture.
    #[test]
    fn the_closed_blob_door_carries_the_apis_sentence_per_posture() {
        use temper_services::services::blob_service::{blob_disabled, blob_disabled_by_policy};
        assert_eq!(
            blob_door(None, false),
            BlobDoor::Closed {
                refusal: blob_disabled().to_string()
            }
        );
        assert_eq!(
            blob_door(None, true),
            BlobDoor::Closed {
                refusal: blob_disabled_by_policy().to_string()
            }
        );

        let mut pairs = AUTH.to_vec();
        pairs.push(("BLOB_STORE_ID", "store_abc123"));
        pairs.push(("BLOB_ENABLED", "false"));
        assert_eq!(
            McpServerConfig::from_lookup(env(&pairs)).unwrap().blob_door,
            BlobDoor::Closed {
                refusal: blob_disabled_by_policy().to_string()
            }
        );
    }

    /// An open door carries the deployment's single-request ceiling.
    #[test]
    fn an_open_blob_door_carries_the_ceiling() {
        let mut pairs = AUTH.to_vec();
        pairs.push(("BLOB_STORE_ID", "store_abc123"));
        pairs.push(("BLOB_SINGLE_REQUEST_MAX_BYTES", "1234"));
        assert_eq!(
            McpServerConfig::from_lookup(env(&pairs)).unwrap().blob_door,
            BlobDoor::Open {
                single_request_max_bytes: 1234
            }
        );
    }
}
