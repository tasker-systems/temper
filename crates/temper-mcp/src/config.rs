use serde::Deserialize;
use std::collections::HashMap;
use std::env;

/// Static MCP server configuration embedded at compile time from `mcp-server.toml`.
static MCP_SERVER_TOML: &str = include_str!("../mcp-server.toml");

/// Top-level shape of `mcp-server.toml`.
#[derive(Debug, Clone, Deserialize)]
struct McpServerFile {
    oauth: OAuthStaticConfig,
}

/// OAuth-related static configuration (allowed redirect URIs, etc.).
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthStaticConfig {
    /// The redirect URIs registration may echo back.
    ///
    /// **On an AS-mode instance this is replaced at boot** by the entry `AS_CLIENTS` holds for this
    /// instance's MCP client id — see [`parse_mcp_config`]. The value compiled in from
    /// `mcp-server.toml` is the external-IdP (Auth0) list, where `AS_CLIENTS` is not the authority
    /// and this file is.
    pub redirect_uris: Vec<String>,
    /// Accept any `http://localhost` or `http://127.0.0.1` redirect URI.
    ///
    /// Survives the AS-mode replacement above. RFC 8252 loopback callbacks use an ephemeral port,
    /// so they cannot be enumerated in any allowlist — desktop and CLI MCP clients depend on this.
    #[serde(default)]
    pub allow_localhost: bool,
}

/// Configuration specific to the MCP server deployment.
///
/// Deliberately carries **no audience**. An instance has exactly one, parsed into
/// `temper_services::auth_config::AuthConfig` and read by both surfaces.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Public base URL of this MCP server, e.g. `https://temperkb.io`.
    /// Used in WWW-Authenticate headers and oauth-protected-resource responses.
    pub mcp_base_url: String,

    /// Pre-registered application client_id for MCP clients.
    /// Returned by the registration endpoint so clients like Claude Desktop
    /// can complete OAuth without manual client_id entry.
    /// `None` if `MCP_CLIENT_ID` is not set — DCR will return 503.
    pub mcp_client_id: Option<String>,

    /// OAuth config: compiled in from `mcp-server.toml`, with the redirect-URI list replaced by
    /// the authoritative one on AS-mode instances.
    pub oauth: OAuthStaticConfig,
}

impl McpConfig {
    pub fn from_env() -> Result<Self, McpConfigError> {
        parse_mcp_config(|k| env::var(k).ok())
    }
}

/// Parse the MCP deployment's configuration, or refuse to produce one.
///
/// The lookup is injected for the same reason `temper_services::auth_config::parse_auth_config`
/// injects its own: an agreement between two environment variables is only worth asserting if the
/// assertion itself is testable without a process environment.
///
/// # The agreement this asserts, and why it is asserted here
///
/// `POST /oauth/register` is deliberately a **thin static-client echo, not RFC 7591 DCR**. It hands
/// back a pre-registered client id and filters the redirect URIs a client proposes against an
/// allowlist. The load-bearing invariant is that **registration never writes to the authorization
/// server's client allowlist**, and that open-redirect protection stays enforced at
/// `/oauth/authorize` against `AS_CLIENTS`. Nothing below relaxes that: this reads `AS_CLIENTS`, and
/// never writes it. A future change that persists client-supplied redirect URIs would reintroduce
/// the redirect-to-code-capture chain and must not be made.
///
/// Two independent statements of one fact used to coexist. `mcp-server.toml` gated what registration
/// echoes; `AS_CLIENTS` gates what `/oauth/authorize` accepts. On an AS-mode instance only the
/// second is authoritative, so the first could drift from it and the symptom was a client that
/// registered successfully and then failed at authorize, one hop from the mistake.
///
/// So on an AS-mode instance the second list stops existing: the allowlist is **derived** from the
/// authoritative one, and a mismatched client id refuses the boot instead of surfacing later.
///
/// AS mode is `AS_ISSUER`'s presence, which is the same signal `parse_auth_config` derives it from.
/// On an external-IdP instance `AS_CLIENTS` is not the authority — Auth0's own callback allowlist is
/// — so the compiled-in list stands and nothing here applies.
///
/// An **unset** `MCP_CLIENT_ID` is not a mismatch. It means this deployment does not offer
/// registration at all, and the endpoint answers `503`; that is a supported posture and boots fine.
pub fn parse_mcp_config(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<McpConfig, McpConfigError> {
    let get = |key: &str| {
        lookup(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };

    let server_file: McpServerFile =
        toml::from_str(MCP_SERVER_TOML).map_err(McpConfigError::Toml)?;
    let mut oauth = server_file.oauth;

    let mcp_base_url = get("MCP_BASE_URL").ok_or(McpConfigError::Missing("MCP_BASE_URL"))?;
    let mcp_client_id = get("MCP_CLIENT_ID");

    let as_mode = get("AS_ISSUER").is_some();
    if let (true, Some(client_id)) = (as_mode, mcp_client_id.as_deref()) {
        let raw = get("AS_CLIENTS").unwrap_or_else(|| "{}".to_string());
        let registry: HashMap<String, Vec<String>> =
            serde_json::from_str(&raw).map_err(|_| McpConfigError::AsClientsMalformed)?;

        let authoritative = registry
            .get(client_id)
            .ok_or(McpConfigError::McpClientIdNotRegistered)?;

        oauth.redirect_uris = authoritative.clone();
    }

    Ok(McpConfig {
        mcp_base_url,
        mcp_client_id,
        oauth,
    })
}

/// Errors that can occur when loading MCP configuration.
///
/// **No message prints a value.** Anyone who can act on one of these can already read the
/// environment, and a config value in a serverless log is a liability with no upside — the same rule
/// `temper_services::auth_config::ConfigError` states for the variables it owns.
#[derive(Debug)]
pub enum McpConfigError {
    /// The variable's name. Carried because the entrypoint aborts on `Display`, and "environment
    /// variable not found" with no name is a remedy an operator cannot act on.
    Missing(&'static str),
    Toml(toml::de::Error),
    AsClientsMalformed,
    McpClientIdNotRegistered,
}

impl std::fmt::Display for McpConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "environment variable {name} is not set."),
            Self::Toml(e) => write!(f, "invalid mcp-server.toml: {e}"),
            Self::AsClientsMalformed => write!(
                f,
                "AS_CLIENTS is not JSON {{clientId: string[]}}. The authorization server parses it \
                 the same way and will refuse it too; fix it there and here at once."
            ),
            Self::McpClientIdNotRegistered => write!(
                f,
                "AS_ISSUER is set, so this instance mints its own tokens — but MCP_CLIENT_ID names \
                 a client that AS_CLIENTS does not register. Registration would succeed and then \
                 /oauth/authorize would refuse that client, one hop from the mistake. Add the \
                 client id to AS_CLIENTS with its redirect URIs, or unset MCP_CLIENT_ID to serve \
                 no registration at all."
            ),
        }
    }
}

impl std::error::Error for McpConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a lookup from pairs. Absent keys return `None`.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    const CLIENT: &str = "temper-mcp";
    const AUTHORITATIVE: &str =
        r#"{"temper-mcp":["https://temper.acme.com/api/auth/mcp-callback"]}"#;

    fn as_mode(pairs: &[(&str, &str)]) -> Vec<(&'static str, String)> {
        let mut out = vec![
            ("MCP_BASE_URL", "https://temper.acme.com".to_string()),
            ("AS_ISSUER", "https://temper.acme.com".to_string()),
        ];
        for (k, v) in pairs {
            out.retain(|(key, _)| key != k);
            out.push((
                Box::leak((*k).to_string().into_boxed_str()) as &'static str,
                (*v).to_string(),
            ));
        }
        out
    }

    fn parse(pairs: Vec<(&'static str, String)>) -> Result<McpConfig, McpConfigError> {
        let owned: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        parse_mcp_config(env(&owned))
    }

    /// The allowlist an AS-mode instance uses is the authoritative one, not the compiled-in one.
    ///
    /// This is the whole point: after this, there is no second list on an AS-mode instance for the
    /// first to drift from. Bitten by deleting the assignment, which leaves `mcp-server.toml`'s
    /// production URIs here and fails this and nothing else.
    #[test]
    fn an_as_instance_takes_its_allowlist_from_the_authoritative_registry() {
        let cfg = parse(as_mode(&[
            ("MCP_CLIENT_ID", CLIENT),
            ("AS_CLIENTS", AUTHORITATIVE),
        ]))
        .expect("a registered client id boots");

        assert_eq!(
            cfg.oauth.redirect_uris,
            vec!["https://temper.acme.com/api/auth/mcp-callback"],
            "the compiled-in list must not survive on an AS-mode instance; it is the second \
             statement of a fact only AS_CLIENTS is authoritative about"
        );
    }

    /// The loopback rule survives the replacement.
    ///
    /// RFC 8252 callbacks use an ephemeral port and cannot be enumerated in any allowlist, so a
    /// replacement that took the whole OAuth block rather than the URI list would break every
    /// desktop and CLI MCP client — silently, since they would simply stop being able to register.
    #[test]
    fn the_loopback_rule_survives_the_replacement() {
        let cfg = parse(as_mode(&[
            ("MCP_CLIENT_ID", CLIENT),
            ("AS_CLIENTS", AUTHORITATIVE),
        ]))
        .expect("a registered client id boots");

        assert!(
            cfg.oauth.allow_localhost,
            "allow_localhost is compiled in and is not a redirect URI; replacing the list must \
             leave it alone"
        );
    }

    /// A client id the authorization server does not register refuses the boot.
    ///
    /// The failure this replaces was a client that registered successfully and was then refused at
    /// `/oauth/authorize` — one hop from the mistake, on a surface whose misconfiguration is
    /// hardest to notice.
    #[test]
    fn an_unregistered_client_id_refuses_the_boot() {
        let err = parse(as_mode(&[
            ("MCP_CLIENT_ID", "typo-in-the-client-id"),
            ("AS_CLIENTS", AUTHORITATIVE),
        ]))
        .expect_err("an unregistered client id must not boot");

        assert!(matches!(err, McpConfigError::McpClientIdNotRegistered));
        let message = err.to_string();
        assert!(
            message.contains("MCP_CLIENT_ID") && message.contains("AS_CLIENTS"),
            "the remedy needs BOTH variable names — the operator has to compare them, and the \
             entrypoint aborts on this string; got: {message}"
        );
        assert!(
            !message.contains("typo-in-the-client-id"),
            "no message prints a value; got: {message}"
        );
    }

    /// An AS-mode instance with no registry at all is the same mistake, not a lenient case.
    ///
    /// Unset `AS_CLIENTS` parses as an empty registry on the authorization server too, so every
    /// client id is unregistered. Reading absence as "skip the check" would have made the emptiest
    /// possible misconfiguration the one case that boots.
    #[test]
    fn an_as_instance_with_no_registry_refuses_a_client_id() {
        let err = parse(as_mode(&[("MCP_CLIENT_ID", CLIENT)]))
            .expect_err("no registry means the client is not registered");

        assert!(matches!(err, McpConfigError::McpClientIdNotRegistered));
    }

    /// No client id is a posture, not a mismatch.
    ///
    /// It means this deployment offers no registration and the endpoint answers `503`. Refusing to
    /// boot on it would turn a supported configuration into an outage.
    #[test]
    fn an_as_instance_with_no_client_id_boots() {
        let cfg = parse(as_mode(&[("AS_CLIENTS", AUTHORITATIVE)]))
            .expect("no client id is a supported posture");

        assert!(cfg.mcp_client_id.is_none());
        assert!(
            !cfg.oauth.redirect_uris.is_empty(),
            "with no client id there is nothing to derive from, so the compiled-in list stands"
        );
    }

    /// On an external-IdP instance the compiled-in list is the authority and `AS_CLIENTS` is not.
    ///
    /// Auth0 holds the callback allowlist there. Deriving from `AS_CLIENTS` — or worse, refusing
    /// the boot over it — would apply an AS instance's rule to an instance that has no AS.
    #[test]
    fn an_external_idp_instance_ignores_the_registry_entirely() {
        let cfg = parse(vec![
            ("MCP_BASE_URL", "https://temperkb.io".to_string()),
            ("MCP_CLIENT_ID", "an-auth0-application".to_string()),
            ("AS_CLIENTS", AUTHORITATIVE.to_string()),
        ])
        .expect("an external-IdP instance does not consult AS_CLIENTS");

        assert!(
            cfg.oauth
                .redirect_uris
                .iter()
                .any(|u| u.contains("temperkb.io")),
            "the compiled-in list must stand where AS_CLIENTS is not the authority"
        );
    }

    /// A registry the authorization server would reject refuses the boot here too.
    ///
    /// Both readers parse the same variable. One accepting what the other refuses is the drift this
    /// module exists to remove, in its smallest form.
    #[test]
    fn a_malformed_registry_refuses_the_boot() {
        let err = parse(as_mode(&[
            ("MCP_CLIENT_ID", CLIENT),
            ("AS_CLIENTS", "not json at all"),
        ]))
        .expect_err("a malformed registry must not boot");

        assert!(matches!(err, McpConfigError::AsClientsMalformed));
    }
}
