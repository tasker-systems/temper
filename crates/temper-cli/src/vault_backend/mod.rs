//! `VaultBackend` — vault-file impl of [`temper_core::operations::Backend`].
//!
//! Per-request construction: CLI action commands build a `VaultBackend` from
//! vault root, manifest, client, and owner, then dispatch one command through it.
//! Each trait method handles vault-file persistence (read/write/delete) and
//! optional push to cloud via the client. Events are synthesized at each step.
//!
//! See `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`.

pub(crate) mod per_doctype;
mod translators;
mod vault_backend;

#[cfg(all(test, feature = "test-db"))]
mod tests;

#[cfg(all(test, feature = "test-db"))]
mod ctx_tests;

pub use vault_backend::{VaultBackend, VaultBackendCtx};

use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::Mutex;

use temper_core::operations::Surface;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Build a tokio runtime + fully-populated [`VaultBackendCtx`] for a Local-mode
/// CLI invocation.
///
/// This is the single assembly point used by all three Local-mode resource
/// arms (delete, update, create). It bundles the runtime with the ctx so
/// callers can `runtime.block_on(backend.<op>(cmd))` against the resulting
/// `VaultBackend` without separately constructing a runtime (which would
/// otherwise force a second client build).
///
/// **Auth-tolerance.** Mirrors [`crate::actions::runtime::publish_local_write_best_effort`]:
/// if the resolved [`temper_client::auth::TokenStore`] holds no token, the
/// returned `ctx.client` is `None` (the user is in offline / not-yet-
/// authenticated mode; the surface can still mutate the vault file and let
/// `temper sync run` reconcile later). Genuine config/network failures from
/// `build_config_store_and_client` bubble.
pub fn assemble_vault_backend(
    config: &Config,
    context: &str,
) -> Result<(Runtime, VaultBackendCtx)> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;

    let (_cfg, store, client) = crate::actions::runtime::build_config_store_and_client()?;
    let client_opt = if store.load().ok().flatten().is_some() {
        Some(Arc::new(client))
    } else {
        // No token resolved (no TEMPER_TOKEN env, no auth.json on disk).
        // Offline path: surfaces still get a backend with vault + manifest;
        // push tail-actions degrade to `PushDeferred { reason: NotAuthenticated }`.
        None
    };

    let device_id = crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());
    let manifest = crate::manifest_io::load_manifest(&config.state_dir, &device_id)?;
    let manifest = Arc::new(Mutex::new(manifest));

    let owner = config.owner_for_context(context);

    let ctx = VaultBackendCtx {
        vault_root: config.vault_root.clone(),
        manifest,
        client: client_opt,
        owner,
        config: Arc::new(config.clone()),
        surface: Surface::CliLocalVault,
    };

    Ok((runtime, ctx))
}
