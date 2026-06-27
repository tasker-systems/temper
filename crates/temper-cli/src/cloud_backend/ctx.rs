//! `CloudBackendCtx` — per-request context for cloud-mode CLI dispatch.
//!
//! Cloud mode has no offline path. If no token resolves,
//! `assemble_cloud_backend` returns an error directing the user
//! to `temper auth login`.

use std::sync::Arc;

use temper_client::TemperClient;
use temper_workflow::operations::Surface;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Per-request context for constructing a [`super::CloudBackend`].
///
/// All fields are public so call-sites can build the struct directly
/// without a further builder method. The ctx struct satisfies the
/// project's "params structs at 5+ args" rule.
pub struct CloudBackendCtx {
    pub client: Arc<TemperClient>,
    pub owner: String,
    /// Context ref string for the create path. Passed verbatim as
    /// `IngestPayload.context_ref` — no synthesis, no sigil-prefixing.
    /// Bare names (no `@`/`+` sigil, no UUID) are intentionally NOT
    /// prefixed with `@me/`; they reach the server's `parse_context_ref`,
    /// which rejects them with `BadRequest`. That server-side rejection IS
    /// the hard-reject for bare names (spec Decision 1).
    pub context_ref: String,
    pub config: Arc<Config>,
    pub surface: Surface,
}

/// Build a tokio runtime + fully-populated [`CloudBackendCtx`] for a
/// cloud-mode CLI invocation.
///
/// **No offline path.** Cloud mode requires a resolved token. If no
/// `TEMPER_TOKEN` env var is set and no token is cached on disk, this
/// function returns an error directing the user to `temper auth login`.
pub fn assemble_cloud_backend(
    config: &Config,
    context: &str,
) -> Result<(tokio::runtime::Runtime, CloudBackendCtx)> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;

    let (_cfg, store, client) = crate::actions::runtime::build_config_store_and_client()?;

    // Cloud mode has no offline path. Require a token or fail with a clear
    // auth error directing the user to authenticate first.
    if store.load().ok().flatten().is_none() {
        return Err(TemperError::Config(
            "not authenticated — run `temper auth login` first".into(),
        ));
    }

    let owner = config.owner_for_context(context);

    // Pass the context value verbatim as the context ref — no synthesis, no
    // `@me/<name>` prefixing. Bare names intentionally reach the server where
    // `parse_context_ref` rejects them with BadRequest (spec Decision 1:
    // bare names are hard-rejected because they are ambiguous in multi-person
    // orgs). Users must supply a decorated ref: `@me/slug`, `@handle/slug`,
    // `+team/slug`, or a UUID.
    let context_ref = context.to_owned();

    let ctx = CloudBackendCtx {
        client: Arc::new(client),
        owner,
        context_ref,
        config: Arc::new(config.clone()),
        surface: Surface::CliCloud,
    };

    Ok((runtime, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(vault_root: &std::path::Path) -> Config {
        Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        }
    }

    #[test]
    fn assemble_cloud_backend_errors_when_no_token() {
        let temp = tempfile::tempdir().unwrap();
        let config = make_config(temp.path());
        let auth_path = temp.path().join("auth.json");
        let nonexistent_config = temp.path().join("no-such-config.toml");

        temp_env::with_vars(
            [
                ("TEMPER_TOKEN", None::<&str>),
                ("TEMPER_AUTH_PATH", Some(auth_path.to_str().unwrap())),
                (
                    "TEMPER_GLOBAL_CONFIG",
                    Some(nonexistent_config.to_str().unwrap()),
                ),
            ],
            || match assemble_cloud_backend(&config, "temper") {
                Ok(_) => panic!("expected Err when no token, got Ok"),
                Err(err) => assert!(
                    format!("{err:?}").contains("temper auth login"),
                    "expected auth-login error, got: {err:?}"
                ),
            },
        );
    }
}
