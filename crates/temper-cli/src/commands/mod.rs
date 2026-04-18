pub mod add;
pub mod auth;
pub mod check;
pub mod config;
pub mod context_cmd;
pub mod doctor;
pub mod events;
pub mod goal;
pub mod graph;
pub mod index;
pub mod init;
pub mod pull;
pub mod remove;
pub mod research;
pub mod resource;
pub mod search_cmd;
pub mod session;
pub mod skill;
pub mod status;
pub mod sync_cmd;
pub mod task;
pub mod team;
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
             To create this context locally: temper context add {context}"
        ));
        Cow::Borrowed("default")
    }
}
