//! Trait-impl tests for `VaultBackend` against a tmp vault.
//!
//! Gated on `test-db` (mirrors Phase 3a's `temper-api/src/backend/tests.rs`).
//! These tests exercise the full `Backend` trait dispatch path through a
//! real vault layout on a tmp filesystem. They do not require a database.

#[cfg(all(test, feature = "test-db"))]
mod show_resource_tests {
    use std::fs;
    use std::sync::Arc;

    use chrono::Utc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use temper_core::operations::{Backend, ResourceRef, ShowResource, Surface};
    use temper_core::types::ids::ResourceId;
    use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};

    use crate::config::Config;
    use crate::vault_backend::{VaultBackend, VaultBackendCtx};

    /// Build a minimal `Config` pointing at `vault_root`.
    fn make_config(vault_root: &std::path::Path) -> Arc<Config> {
        Arc::new(Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        })
    }

    /// Build a `VaultBackend` with `client: None` against a tmp vault root.
    fn make_backend(vault_root: &std::path::Path, manifest: Manifest) -> VaultBackend {
        let config = make_config(vault_root);
        VaultBackend::new(VaultBackendCtx {
            vault_root: vault_root.to_path_buf(),
            manifest: Arc::new(Mutex::new(manifest)),
            client: None,
            owner: "@me".to_string(),
            config,
            surface: Surface::CliLocalVault,
        })
    }

    /// Write a minimal task `.md` file to `path` with the given UUID and title.
    fn write_task_file(path: &std::path::Path, id: &ResourceId, title: &str, slug: &str) {
        let content = format!(
            "---\ntemper-id: \"{}\"\ntemper-type: task\ntemper-context: temper\ntemper-title: '{}'\ntemper-slug: {}\ntemper-stage: backlog\n---\n\nBody content.\n",
            **id, title, slug
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Build a manifest entry for `rel_path` (relative to vault root).
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

    // ── show_resource_uuid_returns_resource_row ───────────────────────────────

    #[tokio::test]
    async fn show_resource_uuid_returns_resource_row() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/my-task.md";
        let abs = tmp.path().join(rel);

        write_task_file(&abs, &id, "My Task", "my-task");

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = ShowResource {
            resource: ResourceRef::Uuid { id },
            origin: Surface::CliLocalVault,
        };

        let output = backend.show_resource(cmd).await.expect("show ok");

        assert_eq!(output.value.title, "My Task");
        assert_eq!(output.value.slug.as_deref(), Some("my-task"));
        assert_eq!(output.value.doc_type_name, "task");
        assert_eq!(output.value.context_name, "temper");
        assert_eq!(output.value.stage.as_deref(), Some("backlog"));
        assert_eq!(output.value.id, id);
        // Read paths emit empty events (Phase 3 precedent).
        assert!(output.events.is_empty(), "read path must emit no events");
    }

    // ── show_resource_scoped_returns_resource_row ─────────────────────────────

    #[tokio::test]
    async fn show_resource_scoped_returns_resource_row() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/scoped-task.md";
        let abs = tmp.path().join(rel);

        write_task_file(&abs, &id, "Scoped Task", "scoped-task");

        // Scoped resolution uses `lookup::find_resource` which reads
        // frontmatter directly — no manifest entry needed.
        let manifest = Manifest::new("test-device".to_string());
        let backend = make_backend(tmp.path(), manifest);

        let cmd = ShowResource {
            resource: ResourceRef::Scoped {
                owner: "@me".to_string(),
                context: "temper".to_string(),
                doctype: "task".to_string(),
                slug: "scoped-task".to_string(),
            },
            origin: Surface::CliLocalVault,
        };

        let output = backend.show_resource(cmd).await.expect("show ok");

        assert_eq!(output.value.title, "Scoped Task");
        assert_eq!(output.value.slug.as_deref(), Some("scoped-task"));
        assert_eq!(output.value.doc_type_name, "task");
        assert!(output.events.is_empty(), "read path must emit no events");
    }

    // ── show_resource_locally_missing_no_client_returns_error ─────────────────

    #[tokio::test]
    async fn show_resource_locally_missing_no_client_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        // Manifest entry present but file does NOT exist on disk.
        let rel = "@me/temper/task/ghost-task.md";
        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = ShowResource {
            resource: ResourceRef::Uuid { id },
            origin: Surface::CliLocalVault,
        };

        let err = backend.show_resource(cmd).await.expect_err("should fail");
        assert!(
            matches!(err, temper_core::error::TemperError::NotFound(_)),
            "expected NotFound, got: {err:?}"
        );
    }

    // ── show_resource_emits_no_events ─────────────────────────────────────────

    #[tokio::test]
    async fn show_resource_emits_no_events() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/event-check.md";
        let abs = tmp.path().join(rel);

        write_task_file(&abs, &id, "Event Check", "event-check");

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = ShowResource {
            resource: ResourceRef::Uuid { id },
            origin: Surface::CliLocalVault,
        };

        let output = backend.show_resource(cmd).await.expect("show ok");
        assert!(
            output.events.is_empty(),
            "show_resource must emit zero events (read path); got: {:?}",
            output.events
        );
    }

    // ── show_resource_client_fallback (stubbed — no test fixture for TemperClient) ──

    /// Client fallback path is not exercised by unit tests because there is no
    /// test fixture for `TemperClient` that can be spun up without a live Auth0
    /// endpoint. This is tracked for a follow-up integration test task.
    ///
    /// TODO: implement this test when a mock or sandbox `TemperClient` fixture
    /// is available (see backlog task `vault-backend-client-fallback-integration-test`).
    #[tokio::test]
    #[ignore = "no TemperClient test fixture available yet; tracked as backlog task"]
    async fn show_resource_locally_missing_with_client_falls_back_to_api() {
        todo!("implement when a mock TemperClient fixture is available")
    }
}
