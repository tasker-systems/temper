//! `VaultBackend` struct + `impl Backend` for vault-file persistence.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use temper_client::TemperClient;
use temper_core::operations::Surface;
use temper_core::types::manifest::Manifest;

use crate::config::Config;

/// Local-file-backed backend impl. Constructed per inbound CLI invocation.
///
/// See `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`.
///
/// Fields exceed the project's 5-arg threshold — `VaultBackendCtx` builder
/// is mandatory for construction. Fields above the project's 5-param threshold;
/// already using a params struct per the design spec.
pub struct VaultBackend {
    vault_root: PathBuf,
    manifest: Arc<Mutex<Manifest>>,
    client: Option<Arc<TemperClient>>,
    /// Owner sigil (e.g. `"@me"` or `"+team-..."`) used for vault path
    /// construction via `Vault::doc_file`. Stored as a string because
    /// `OwnerHandle` does not yet exist in `temper-core` (aspirational in
    /// the spec; deferred to a future refactor).
    owner: String,
    config: Arc<Config>,
    /// Origin of the inbound command. Today always `CliLocalVault`; stored
    /// for forward-compat (Phase 6 telemetry/event tagging).
    #[expect(dead_code, reason = "stored for Phase 6 telemetry; not yet consumed")]
    surface: Surface,
}

/// Builder / context for constructing a `VaultBackend`.
///
/// All fields are public so call-sites can build the struct directly without a
/// further builder method. The ctx struct is the 6-field params struct required
/// by the project's "params structs at 5+ args" rule.
pub struct VaultBackendCtx {
    pub vault_root: PathBuf,
    pub manifest: Arc<Mutex<Manifest>>,
    pub client: Option<Arc<TemperClient>>,
    pub owner: String,
    pub config: Arc<Config>,
    pub surface: Surface,
}

impl VaultBackend {
    /// Construct from a fully-populated `VaultBackendCtx`.
    pub fn new(ctx: VaultBackendCtx) -> Self {
        Self {
            vault_root: ctx.vault_root,
            manifest: ctx.manifest,
            client: ctx.client,
            owner: ctx.owner,
            config: ctx.config,
            surface: ctx.surface,
        }
    }

    #[expect(
        dead_code,
        reason = "used by Tasks 3+ (resolve_resource_ref); \
                  remove suppression when Task 3 lands"
    )]
    pub(crate) fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    #[expect(
        dead_code,
        reason = "used by Tasks 5+ (show_resource); \
                  remove suppression when Task 5 lands"
    )]
    pub(crate) fn manifest(&self) -> &Arc<Mutex<Manifest>> {
        &self.manifest
    }

    #[expect(
        dead_code,
        reason = "used by Tasks 7+ (create_resource push tail); \
                  remove suppression when Task 7 lands"
    )]
    pub(crate) fn client(&self) -> Option<&Arc<TemperClient>> {
        self.client.as_ref()
    }

    // Dead in lib target; called from tests only in Task 2. Real callers land
    // in Task 3+. Remove the cfg_attr suppression when Task 3 lands.
    #[cfg_attr(not(test), expect(dead_code, reason = "lib callers land in Task 3+"))]
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    #[expect(
        dead_code,
        reason = "used by Tasks 3+ (resolve_resource_ref → lookup::find_resource); \
                  remove suppression when Task 3 lands"
    )]
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_ctx() -> VaultBackendCtx {
        let tmp = tempfile::tempdir().unwrap();
        let vault_root = tmp.path().to_path_buf();
        let manifest = Arc::new(Mutex::new(Manifest::new("test-device".to_string())));
        let config = Arc::new(Config {
            vault_root: vault_root.clone(),
            state_dir: vault_root.join(".temper"),
            contexts: vec![],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        });
        VaultBackendCtx {
            vault_root,
            manifest,
            client: None,
            owner: "@me".to_string(),
            config,
            surface: Surface::CliLocalVault,
        }
    }

    #[test]
    fn vault_backend_new_constructs_from_ctx() {
        let ctx = make_test_ctx();
        let backend = VaultBackend::new(ctx);
        assert_eq!(backend.owner(), "@me");
    }
}
