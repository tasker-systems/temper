pub mod add;
pub mod auth;
pub mod check;
pub mod context_cmd;
pub mod doctor;
pub mod events;
pub mod goal;
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
        temper_client::error::ClientError::SystemAccessRequired {
            email,
            display_name,
            access_mode,
            join_request_status,
            request_url,
            cli_command,
        } => crate::error::TemperError::SystemAccessRequired(Box::new(
            temper_core::error::CliAccessDetails {
                email,
                display_name,
                access_mode,
                join_request_status,
                request_url,
                cli_command,
            },
        )),
        other => crate::error::TemperError::Api(other.to_string()),
    }
}

/// Resolve a context name, falling back to "default" with a warning if the
/// context directory doesn't exist in the vault.
pub fn resolve_context_with_fallback<'a>(
    config: &crate::config::Config,
    context: &'a str,
) -> Cow<'a, str> {
    let ctx_dir = config.vault_root.join(context);
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
