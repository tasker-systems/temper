pub mod add;
pub mod auth;
pub mod check;
pub mod config;
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
pub mod warmup;

use std::borrow::Cow;

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
