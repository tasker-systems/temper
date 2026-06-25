//! Backend selection — the single helper surfaces use to acquire a
//! `Box<dyn Backend>`.
//!
//! temper is cloud-only: every surface dispatches writes through
//! `CloudBackend`. Surfaces never instantiate `CloudBackend` directly;
//! they always go through this helper.
//!
//! See `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`.

use std::sync::Arc;

use tokio::runtime::Runtime;

use temper_client::TemperClient;
use temper_workflow::operations::Backend;

use crate::config::Config;
use crate::error::Result;

/// Build a tokio runtime + `Box<dyn Backend>` + `Arc<TemperClient>` for a CLI invocation.
///
/// Always returns `CloudBackend` via `assemble_cloud_backend`, which
/// errors if no token resolves — temper is cloud-only and has no offline
/// write path. In no-embed builds, `CloudBackend`'s methods return
/// `BadRequest`.
///
/// **Why bundle the runtime:** `assemble_cloud_backend` constructs a
/// runtime, then builds the client on it. Returning both as a tuple
/// gives surfaces one `block_on` handle without constructing a second
/// runtime by accident.
///
/// The returned `Arc<TemperClient>` is the same client the backend dispatches
/// through; surfaces use it for the post-write projection refresh.
pub fn build_backend(
    config: &Config,
    ctx: &str,
) -> Result<(Runtime, Box<dyn Backend>, Arc<TemperClient>)> {
    let (runtime, backend_ctx) = crate::cloud_backend::assemble_cloud_backend(config, ctx)?;
    // Clone the `Arc` out before `CloudBackend::new` consumes the ctx —
    // surfaces use it for the post-write projection refresh.
    let client = Arc::clone(&backend_ctx.client);
    let backend: Box<dyn Backend> = Box::new(crate::cloud_backend::CloudBackend::new(backend_ctx));
    Ok((runtime, backend, client))
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
    fn build_backend_errors_without_a_token() {
        // temper is cloud-only — a write backend requires a resolved
        // token. With no token, `build_backend` must fail fast with a
        // clear `temper auth login` directive (before any network call).
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
            || {
                let err = build_backend(&config, "temper")
                    .map(|_| ())
                    .expect_err("no token must error");
                let msg = format!("{err:?}");
                assert!(
                    msg.contains("temper auth login")
                        || msg.contains("TEMPER_TOKEN")
                        || msg.contains("authenticated"),
                    "expected an auth error, got: {err:?}"
                );
            },
        );
    }
}
