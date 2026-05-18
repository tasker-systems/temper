//! Tests for `assemble_vault_backend` — gated on `test-db` to match the
//! sibling `tests` module's gate (the helper itself is config-agnostic, but
//! these tests require Config construction against a tmp filesystem).

use chrono::Utc;
use tempfile::tempdir;
use uuid::Uuid;

use temper_core::operations::Surface;
use temper_core::types::ids::ResourceId;
use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};

use crate::config::Config;
use crate::manifest_io;
use crate::vault_backend::assemble_vault_backend;

/// Build a minimal `Config` rooted at `vault_root`. Mirrors
/// `vault_backend::tests::show_resource_tests::make_config` but unwrapped
/// (we want an owned `Config` to pass by reference, not an `Arc`).
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

/// Build a manifest entry for `rel_path`. Same shape as the sibling
/// `tests::show_resource_tests::make_manifest_entry`.
fn make_manifest_entry(rel_path: &str) -> ManifestEntry {
    ManifestEntry {
        path: rel_path.to_string(),
        body_hash: "sha256:abc".to_string(),
        remote_body_hash: "sha256:abc".to_string(),
        managed_hash: String::new(),
        open_hash: String::new(),
        remote_managed_hash: String::new(),
        remote_open_hash: String::new(),
        synced_at: Utc::now(),
        state: ManifestEntryState::Clean,
        mtime_secs: None,
        provisional: false,
        last_audit_id: None,
    }
}

/// Env-isolation envelope used by every test in this module.
///
/// Forces Local mode and points `TEMPER_AUTH_PATH` at a non-existent file in
/// the per-test tmpdir so the disk store finds no token. Also points
/// `TEMPER_GLOBAL_CONFIG` at a non-existent file so cloud-config loading
/// falls through to defaults — matching `actions::runtime::tests::publish_best_effort_returns_ok_none_when_no_token`.
fn with_local_no_auth<F: FnOnce()>(tmp: &std::path::Path, f: F) {
    let auth_path = tmp.join("auth.json");
    let nonexistent_config = tmp.join("no-such-config.toml");
    temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("local")),
            ("TEMPER_TOKEN", None),
            ("TEMPER_AUTH_PATH", Some(auth_path.to_str().unwrap())),
            (
                "TEMPER_GLOBAL_CONFIG",
                Some(nonexistent_config.to_str().unwrap()),
            ),
        ],
        f,
    );
}

#[test]
fn assemble_vault_backend_populates_all_fields_against_tmp_dir() {
    let dir = tempdir().unwrap();
    let config = make_config(dir.path());

    with_local_no_auth(dir.path(), || {
        let (runtime, ctx) = assemble_vault_backend(&config, "temper").expect("assemble ok");

        assert_eq!(ctx.vault_root, config.vault_root, "vault_root copied");
        assert_eq!(
            ctx.surface,
            Surface::CliLocalVault,
            "surface fixed to Local"
        );
        assert_eq!(
            ctx.owner,
            config.owner_for_context("temper"),
            "owner resolved from context"
        );
        assert!(!ctx.owner.is_empty(), "owner non-empty (default \"@me\")");
        assert_eq!(
            ctx.config.vault_root, config.vault_root,
            "ctx.config holds the same vault_root"
        );

        // Drop the runtime explicitly so the tokio runtime tears down before
        // tempdir cleanup; tempdir's Drop only runs once `dir` falls out of
        // scope, but explicit drop here keeps test ordering predictable.
        drop(runtime);
    });
}

#[test]
fn assemble_vault_backend_loads_manifest_from_state_dir() {
    let dir = tempdir().unwrap();
    let config = make_config(dir.path());

    // Pre-populate the manifest with one entry. `manifest_io::save_manifest`
    // creates the state_dir for us.
    let rid = ResourceId::from(Uuid::now_v7());
    let mut seeded = Manifest::new("test-device".to_string());
    seeded
        .entries
        .insert(rid, make_manifest_entry("@me/temper/task/seed.md"));
    manifest_io::save_manifest(&config.state_dir, &seeded).expect("seed manifest written");

    with_local_no_auth(dir.path(), || {
        let (runtime, ctx) = assemble_vault_backend(&config, "temper").expect("assemble ok");

        let guard = runtime.block_on(ctx.manifest.lock());
        assert_eq!(guard.entries.len(), 1, "loaded the seeded entry");
        let loaded_entry = guard.entries.get(&rid).expect("rid present");
        assert_eq!(loaded_entry.path, "@me/temper/task/seed.md");
        drop(guard);

        drop(runtime);
    });
}

#[test]
fn assemble_vault_backend_yields_none_client_when_no_token() {
    let dir = tempdir().unwrap();
    let config = make_config(dir.path());

    with_local_no_auth(dir.path(), || {
        let (runtime, ctx) = assemble_vault_backend(&config, "temper").expect("assemble ok");

        assert!(
            ctx.client.is_none(),
            "no auth.json + no TEMPER_TOKEN must yield client: None"
        );
        drop(runtime);
    });
}
