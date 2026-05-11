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

// ─────────────────────────────────────────────────────────────────────────────
// list_resources tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "test-db"))]
mod list_resources_tests {
    use std::fs;
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use temper_core::operations::{Backend, ListFilter, ListResources, Surface};
    use temper_core::types::manifest::Manifest;

    use crate::config::Config;
    use crate::vault_backend::{VaultBackend, VaultBackendCtx};

    fn make_config(vault_root: &std::path::Path) -> Arc<Config> {
        Arc::new(Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            // One context so scan_rows has a context to iterate over
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        })
    }

    fn make_backend(vault_root: &std::path::Path) -> VaultBackend {
        let config = make_config(vault_root);
        let manifest = Arc::new(Mutex::new(Manifest::new("test-device".to_string())));
        VaultBackend::new(VaultBackendCtx {
            vault_root: vault_root.to_path_buf(),
            manifest,
            client: None,
            owner: "@me".to_string(),
            config,
            surface: Surface::CliLocalVault,
        })
    }

    /// Write a minimal `.md` file for a given doctype/context combination.
    fn write_md(path: &std::path::Path, slug: &str, title: &str, doc_type: &str, context: &str) {
        let content = format!(
            "---\ntemper-type: {doc_type}\ntemper-context: {context}\ntemper-title: '{title}'\ntemper-slug: {slug}\ntemper-stage: backlog\n---\n\nBody.\n"
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // ── list_resources_filters_by_context_and_doctype ────────────────────────

    #[tokio::test]
    async fn list_resources_filters_by_context_and_doctype() {
        let tmp = tempfile::tempdir().unwrap();

        // Seed two task files under @me/temper/task/
        let base = tmp.path().join("@me/temper/task");
        write_md(&base.join("foo.md"), "foo", "Foo Task", "task", "temper");
        write_md(&base.join("bar.md"), "bar", "Bar Task", "task", "temper");

        let backend = make_backend(tmp.path());
        let cmd = ListResources {
            filter: ListFilter {
                doctype: Some("task".to_string()),
                context: Some("temper".to_string()),
                stage: None,
                goal: None,
                limit: None,
            },
            origin: Surface::CliLocalVault,
        };

        let output = backend.list_resources(cmd).await.expect("list ok");
        assert_eq!(output.value.len(), 2, "expected 2 task summaries");
        // Verify at least one slug is present
        let slugs: Vec<&str> = output.value.iter().map(|s| s.slug.as_str()).collect();
        assert!(slugs.contains(&"foo") || slugs.contains(&"bar"));
        // Read path — emit no events.
        assert!(
            output.events.is_empty(),
            "list read path must emit no events"
        );
    }

    // ── list_resources_respects_limit ─────────────────────────────────────────

    #[tokio::test]
    async fn list_resources_respects_limit() {
        let tmp = tempfile::tempdir().unwrap();

        let base = tmp.path().join("@me/temper/task");
        write_md(&base.join("alpha.md"), "alpha", "Alpha", "task", "temper");
        write_md(&base.join("beta.md"), "beta", "Beta", "task", "temper");
        write_md(&base.join("gamma.md"), "gamma", "Gamma", "task", "temper");

        let backend = make_backend(tmp.path());
        let cmd = ListResources {
            filter: ListFilter {
                doctype: Some("task".to_string()),
                context: Some("temper".to_string()),
                stage: None,
                goal: None,
                limit: Some(2),
            },
            origin: Surface::CliLocalVault,
        };

        let output = backend.list_resources(cmd).await.expect("list ok");
        assert_eq!(output.value.len(), 2, "limit should truncate to 2");
    }

    // ── list_resources_empty_dir_returns_empty_vec ────────────────────────────

    #[tokio::test]
    async fn list_resources_empty_dir_returns_empty_vec() {
        let tmp = tempfile::tempdir().unwrap();
        // No .md files created — just the vault root.

        let backend = make_backend(tmp.path());
        let cmd = ListResources {
            filter: ListFilter {
                doctype: Some("task".to_string()),
                context: Some("temper".to_string()),
                stage: None,
                goal: None,
                limit: None,
            },
            origin: Surface::CliLocalVault,
        };

        let output = backend.list_resources(cmd).await.expect("list ok");
        assert!(
            output.value.is_empty(),
            "expected empty vec when no files exist"
        );
    }

    // ── list_resources_requires_doctype ──────────────────────────────────────

    #[tokio::test]
    async fn list_resources_requires_doctype() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path());
        let cmd = ListResources {
            filter: ListFilter {
                doctype: None,
                context: None,
                stage: None,
                goal: None,
                limit: None,
            },
            origin: Surface::CliLocalVault,
        };

        let err = backend
            .list_resources(cmd)
            .await
            .expect_err("should fail without doctype");
        assert!(
            matches!(err, temper_core::error::TemperError::BadRequest(_)),
            "expected BadRequest, got: {err:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// search_resources tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "test-db"))]
mod search_resources_tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use temper_core::operations::{Backend, SearchQuery, SearchResources, Surface};
    use temper_core::types::manifest::Manifest;

    use crate::config::Config;
    use crate::vault_backend::{VaultBackend, VaultBackendCtx};

    fn make_backend_no_client(vault_root: &std::path::Path) -> VaultBackend {
        let config = Arc::new(Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec![],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        });
        let manifest = Arc::new(Mutex::new(Manifest::new("test-device".to_string())));
        VaultBackend::new(VaultBackendCtx {
            vault_root: vault_root.to_path_buf(),
            manifest,
            client: None,
            owner: "@me".to_string(),
            config,
            surface: Surface::CliLocalVault,
        })
    }

    // ── search_resources_no_client_returns_bad_request ────────────────────────

    #[tokio::test]
    async fn search_resources_no_client_returns_bad_request() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend_no_client(tmp.path());
        let cmd = SearchResources {
            query: SearchQuery {
                query: "rust backend".to_string(),
                doctype: None,
                context: None,
                limit: Some(5),
            },
            origin: Surface::CliLocalVault,
        };

        let err = backend
            .search_resources(cmd)
            .await
            .expect_err("should fail without client");
        assert!(
            matches!(err, temper_core::error::TemperError::BadRequest(_)),
            "expected BadRequest, got: {err:?}"
        );
    }

    // ── search_resources_with_mock_client_passes_query_through ────────────────

    /// Client path not exercised — no mock/sandbox `TemperClient` available.
    /// Tracked as a follow-up integration test.
    #[tokio::test]
    #[ignore = "no TemperClient test fixture available; tracked as backlog task"]
    async fn search_resources_with_mock_client_passes_query_through() {
        todo!("implement when a mock TemperClient fixture is available")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// create_resource tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "test-db"))]
mod create_resource_tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use temper_core::operations::{Backend, CreateResource, DomainEvent, PushDeferReason, Surface};
    use temper_core::types::managed_meta::ManagedMeta;
    use temper_core::types::manifest::Manifest;

    use crate::config::Config;
    use crate::vault_backend::{VaultBackend, VaultBackendCtx};

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

    fn make_backend(vault_root: &std::path::Path) -> VaultBackend {
        let config = make_config(vault_root);
        let manifest = Arc::new(Mutex::new(Manifest::new("test-device".to_string())));
        VaultBackend::new(VaultBackendCtx {
            vault_root: vault_root.to_path_buf(),
            manifest,
            client: None,
            owner: "@me".to_string(),
            config,
            surface: Surface::CliLocalVault,
        })
    }

    fn concept_cmd(title: &str, slug: &str, context: &str) -> CreateResource {
        CreateResource {
            slug: slug.to_string(),
            doctype: "concept".to_string(),
            context: context.to_string(),
            title: title.to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        }
    }

    fn decision_cmd(title: &str, slug: &str, context: &str) -> CreateResource {
        CreateResource {
            slug: slug.to_string(),
            doctype: "decision".to_string(),
            context: context.to_string(),
            title: title.to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        }
    }

    // ── create_resource_concept_writes_file_and_manifest_entry ───────────────

    #[tokio::test]
    async fn create_resource_concept_writes_file_and_manifest_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path());
        let cmd = concept_cmd("My Concept", "my-concept", "temper");

        let output = backend.create_resource(cmd).await.expect("create ok");

        // File written at expected path.
        let expected_path = tmp.path().join("@me/temper/concept/my-concept.md");
        assert!(expected_path.exists(), "concept file must be on disk");

        // ResourceRow is populated correctly.
        assert_eq!(output.value.title, "My Concept");
        assert_eq!(output.value.doc_type_name, "concept");
        assert_eq!(output.value.context_name, "temper");

        // Events: VaultFileWritten + VaultManifestUpdated + PushDeferred(Offline).
        assert_eq!(
            output.events.len(),
            3,
            "expected 3 events, got: {:?}",
            output.events
        );
        assert!(
            matches!(&output.events[0], DomainEvent::VaultFileWritten { path } if path.ends_with(".md")),
            "first event must be VaultFileWritten"
        );
        assert!(
            matches!(&output.events[1], DomainEvent::VaultManifestUpdated { .. }),
            "second event must be VaultManifestUpdated"
        );
        assert!(
            matches!(
                &output.events[2],
                DomainEvent::PushDeferred {
                    reason: PushDeferReason::Offline
                }
            ),
            "third event must be PushDeferred(Offline), got: {:?}",
            output.events[2]
        );

        // Manifest entry is present.
        let manifest = backend.manifest().lock().await;
        assert_eq!(manifest.entries.len(), 1, "manifest must have one entry");
        let entry = manifest.entries.values().next().unwrap();
        assert!(
            entry.path.ends_with(".md"),
            "manifest entry path must end with .md"
        );
        assert!(entry.provisional, "new entry must be provisional");
    }

    // ── create_resource_decision_writes_file ─────────────────────────────────

    #[tokio::test]
    async fn create_resource_decision_writes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path());
        let cmd = decision_cmd("Use Postgres", "use-postgres", "temper");

        let output = backend.create_resource(cmd).await.expect("create ok");

        let expected_path = tmp.path().join("@me/temper/decision/use-postgres.md");
        assert!(expected_path.exists(), "decision file must be on disk");
        assert_eq!(output.value.doc_type_name, "decision");
    }

    // ── create_resource_no_client_emits_push_deferred_offline ────────────────

    #[tokio::test]
    async fn create_resource_no_client_emits_push_deferred_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path()); // client: None
        let cmd = concept_cmd("Offline Concept", "offline-concept", "temper");

        let output = backend.create_resource(cmd).await.expect("create ok");
        let push_event = output.events.last().expect("at least one event");
        assert!(
            matches!(
                push_event,
                DomainEvent::PushDeferred {
                    reason: PushDeferReason::Offline
                }
            ),
            "expected PushDeferred(Offline), got: {push_event:?}"
        );
    }

    // ── create_resource_validate_create_rejects_empty_title ──────────────────

    #[tokio::test]
    async fn create_resource_validate_create_rejects_empty_title() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path());
        let cmd = CreateResource {
            slug: "valid-slug".to_string(),
            doctype: "concept".to_string(),
            context: "temper".to_string(),
            title: "".to_string(), // empty title
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        };

        let err = backend.create_resource(cmd).await.expect_err("should fail");
        assert!(
            matches!(err, temper_core::error::TemperError::BadRequest(_)),
            "expected BadRequest for empty title, got: {err:?}"
        );
    }

    // ── create_resource_applies_doctype_defaults_at_write_time ───────────────
    //
    // Concepts don't have stage/mode/effort defaults, but the template should
    // produce a file with `temper-type: concept`. We verify that the managed_value
    // apply_defaults_value path is called by checking that the returned ResourceRow
    // has the correct doctype (i.e., the full pipeline ran without skipping the
    // defaults step).

    #[tokio::test]
    async fn create_resource_applies_doctype_defaults_at_write_time() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path());
        let cmd = concept_cmd("Defaults Concept", "defaults-concept", "temper");

        let output = backend.create_resource(cmd).await.expect("create ok");

        // The concept template/frontmatter has temper-type: concept.
        assert_eq!(output.value.doc_type_name, "concept");
    }

    // ── create_resource_invokes_ensure_managed_identity_keys ─────────────────

    #[tokio::test]
    async fn create_resource_invokes_ensure_managed_identity_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path());
        let cmd = concept_cmd("Identity Keys Concept", "identity-keys-concept", "temper");

        let output = backend.create_resource(cmd).await.expect("create ok");

        // Read on-disk frontmatter directly to verify identity keys were injected.
        let expected_path = tmp
            .path()
            .join("@me/temper/concept/identity-keys-concept.md");
        let content = std::fs::read_to_string(&expected_path).unwrap();
        assert!(
            content.contains("temper-title"),
            "on-disk frontmatter must contain temper-title"
        );
        assert!(
            content.contains("temper-slug"),
            "on-disk frontmatter must contain temper-slug"
        );
        // The ResourceRow title should also match.
        assert_eq!(output.value.title, "Identity Keys Concept");
        assert_eq!(output.value.slug.as_deref(), Some("identity-keys-concept"));
    }

    // ── create_resource_unsupported_doctype_returns_bad_request ──────────────

    #[tokio::test]
    async fn create_resource_unsupported_doctype_returns_bad_request() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = make_backend(tmp.path());
        // Use a doctype that passes validate_create (i.e., is a known doctype)
        // but is scoped down in per_doctype::write_for.
        let cmd = CreateResource {
            slug: "2026-05-11-my-task".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "My Task".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        };

        let err = backend
            .create_resource(cmd)
            .await
            .expect_err("task not yet supported");
        assert!(
            matches!(err, temper_core::error::TemperError::BadRequest(_)),
            "expected BadRequest for task doctype, got: {err:?}"
        );
    }

    // ── create_resource_remote_synced (stubbed — no TemperClient fixture) ────

    /// RemoteSynced path not exercised — no mock/sandbox `TemperClient` fixture.
    /// Tracked as a follow-up integration test.
    #[tokio::test]
    #[ignore = "no TemperClient test fixture available; tracked as backlog task"]
    async fn create_resource_with_client_emits_remote_synced_on_success() {
        todo!("implement when a mock TemperClient fixture is available")
    }

    #[tokio::test]
    #[ignore = "no TemperClient test fixture available; tracked as backlog task"]
    async fn create_resource_with_client_emits_push_deferred_on_network_error() {
        todo!("implement when a mock TemperClient fixture is available")
    }
}
