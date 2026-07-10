pub mod admin;
pub mod admin_machine;
pub mod admin_saml;
pub mod auth;
pub mod check;
pub mod cogmap;
pub mod config;
pub mod context_cmd;
pub mod edge;
pub mod facet;
pub mod init;
pub mod invitations;
pub mod invocation;
pub mod pull;
pub mod resource;
pub mod search_cmd;
pub mod skill;
pub mod status;
pub mod steward;
pub mod task;
pub mod team;
pub mod version;
pub mod warmup;

use std::borrow::Cow;

/// Convert a ClientError to a TemperError, preserving SystemAccessRequired details.
pub fn client_err(e: temper_client::error::ClientError) -> crate::error::TemperError {
    match e {
        temper_client::error::ClientError::SystemAccessRequired(details) => {
            crate::error::TemperError::SystemAccessRequired(details)
        }
        other => crate::error::TemperError::Api(other.to_string()),
    }
}

/// Resolve a context name, falling back to "default" with a warning if the
/// context directory doesn't exist in the vault.
///
/// Checks for the context under its owner-scoped path
/// (`<vault_root>/<owner>/<context>/`), not the legacy flat layout.
pub fn resolve_context_with_fallback<'a>(
    config: &crate::config::Config,
    context: &'a str,
) -> Cow<'a, str> {
    let owner = config.owner_for_context(context);
    let ctx_dir = config.vault_root.join(&owner).join(context);
    if ctx_dir.exists() {
        Cow::Borrowed(context)
    } else {
        crate::output::warning(format!(
            "Context \"{context}\" not found in vault. Using \"default\" context.\n  \
             To subscribe to this context locally: temper context subscribe {context}"
        ));
        Cow::Borrowed("default")
    }
}
