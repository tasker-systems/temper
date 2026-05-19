//! Backend selection — single helper surfaces use to acquire a
//! `Box<dyn Backend>` based on `VaultState::from_env()`.
//!
//! Surfaces never instantiate `VaultBackend` or `CloudBackend` directly;
//! they always go through this helper. The result is a `Box<dyn Backend>`
//! that surfaces dispatch one command through — no per-mode code at the
//! surface level.
//!
//! See `docs/superpowers/specs/2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`.

use tokio::runtime::Runtime;

use temper_core::operations::Backend;
use temper_core::types::config::VaultState;

use crate::config::Config;
use crate::error::Result;

/// Build a tokio runtime + `Box<dyn Backend>` selected by the current
/// `VaultState`.
///
/// - `VaultState::Local`: returns `VaultBackend` via `assemble_vault_backend`.
///   Tolerates a missing token (offline path).
/// - `VaultState::Cloud`: returns `CloudBackend` via `assemble_cloud_backend`.
///   Errors out if no token resolves — cloud mode has no offline path.
///   In no-embed builds, CloudBackend's methods return `BadRequest`.
///
/// **Why bundle the runtime:** both `assemble_*` functions construct a
/// runtime, then build their clients on it. Returning both as a tuple
/// gives surfaces one `block_on` handle without constructing a second
/// runtime by accident.
pub fn build_backend(config: &Config, ctx: &str) -> Result<(Runtime, Box<dyn Backend>)> {
    match VaultState::from_env() {
        VaultState::Local => {
            let (runtime, backend_ctx) = crate::vault_backend::assemble_vault_backend(config, ctx)?;
            let backend: Box<dyn Backend> =
                Box::new(crate::vault_backend::VaultBackend::new(backend_ctx));
            Ok((runtime, backend))
        }
        VaultState::Cloud => {
            let (runtime, backend_ctx) = crate::cloud_backend::assemble_cloud_backend(config, ctx)?;
            let backend: Box<dyn Backend> =
                Box::new(crate::cloud_backend::CloudBackend::new(backend_ctx));
            Ok((runtime, backend))
        }
    }
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
    fn build_backend_local_mode_succeeds_when_state_is_local() {
        let temp = tempfile::tempdir().unwrap();
        let config = make_config(temp.path());
        let auth_path = temp.path().join("auth.json");
        let nonexistent_config = temp.path().join("no-such-config.toml");

        temp_env::with_vars(
            [
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None::<&str>),
                ("TEMPER_AUTH_PATH", Some(auth_path.to_str().unwrap())),
                (
                    "TEMPER_GLOBAL_CONFIG",
                    Some(nonexistent_config.to_str().unwrap()),
                ),
            ],
            || {
                let result = build_backend(&config, "temper");
                assert!(
                    result.is_ok(),
                    "local-mode build_backend should succeed without a token, got: {:?}",
                    result.err()
                );
            },
        );
    }
}
