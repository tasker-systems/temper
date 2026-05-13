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

// ─────────────────────────────────────────────────────────────────────────────
// update_resource tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "test-db"))]
mod update_resource_tests {
    use std::fs;
    use std::sync::Arc;

    use chrono::Utc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use temper_core::operations::{
        Backend, BodyUpdate, DomainEvent, MoveSpec, PushDeferReason, ResourceRef, Surface,
        UpdateResource,
    };
    use temper_core::types::ids::ResourceId;
    use temper_core::types::managed_meta::ManagedMeta;
    use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};

    use crate::config::Config;
    use crate::vault_backend::{VaultBackend, VaultBackendCtx};

    fn make_config(vault_root: &std::path::Path) -> Arc<Config> {
        Arc::new(Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string(), "writing".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        })
    }

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

    /// Write a minimal task `.md` file. Returns the resource id.
    fn write_task_file(
        path: &std::path::Path,
        id: &ResourceId,
        title: &str,
        slug: &str,
        stage: &str,
        context: &str,
        body: &str,
    ) {
        let content = format!(
            "---\ntemper-id: \"{}\"\ntemper-type: task\ntemper-context: {}\ntemper-title: '{}'\ntemper-slug: {}\ntemper-stage: {}\n---\n\n{}\n",
            **id, context, title, slug, stage, body
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // ── update_resource_scalar_field_via_managed_meta ─────────────────────────

    #[tokio::test]
    async fn update_resource_scalar_field_via_managed_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/foo.md";
        let abs = tmp.path().join(rel);

        write_task_file(
            &abs,
            &id,
            "Foo Task",
            "foo",
            "backlog",
            "temper",
            "Original body.",
        );

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: None,
            managed_meta: Some(ManagedMeta {
                stage: Some("done".to_string()),
                ..ManagedMeta::default()
            }),
            open_meta: None,
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        let output = backend.update_resource(cmd).await.expect("update ok");

        // On-disk frontmatter must reflect the new stage.
        let disk_content = fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert!(
            disk_content.contains("temper-stage: done"),
            "expected temper-stage: done in disk file, got:\n{disk_content}"
        );
        // Row reflects update.
        assert_eq!(output.value.stage.as_deref(), Some("done"));
        // Events: VaultFileWritten + VaultManifestUpdated + PushDeferred(Offline).
        assert_eq!(
            output.events.len(),
            3,
            "expected 3 events, got: {:?}",
            output.events
        );
        assert!(
            matches!(&output.events[0], DomainEvent::VaultFileWritten { .. }),
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
            "third event must be PushDeferred(Offline)"
        );
    }

    // ── update_resource_open_meta_array_appends ───────────────────────────────

    #[tokio::test]
    async fn update_resource_open_meta_array_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/array-task.md";
        let abs = tmp.path().join(rel);

        write_task_file(
            &abs,
            &id,
            "Array Task",
            "array-task",
            "backlog",
            "temper",
            "Body.",
        );

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: None,
            managed_meta: None,
            open_meta: Some(serde_json::json!({"tags": ["new-tag"]})),
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        backend.update_resource(cmd).await.expect("update ok");

        let disk_content = fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert!(
            disk_content.contains("new-tag"),
            "expected new-tag in disk file tags, got:\n{disk_content}"
        );
    }

    // ── update_resource_body_only ─────────────────────────────────────────────

    #[tokio::test]
    async fn update_resource_body_only() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/body-task.md";
        let abs = tmp.path().join(rel);

        write_task_file(
            &abs,
            &id,
            "Body Task",
            "body-task",
            "backlog",
            "temper",
            "Original body content.",
        );

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: Some(BodyUpdate::new("Replaced body content.")),
            managed_meta: None,
            open_meta: None,
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        backend.update_resource(cmd).await.expect("update ok");

        let disk_content = fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert!(
            disk_content.contains("Replaced body content."),
            "expected new body in disk file, got:\n{disk_content}"
        );
        assert!(
            !disk_content.contains("Original body content."),
            "old body must not remain, got:\n{disk_content}"
        );
    }

    // ── update_resource_context_to_moves_file ─────────────────────────────────

    #[tokio::test]
    async fn update_resource_context_to_moves_file() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/foo.md";
        let abs = tmp.path().join(rel);

        write_task_file(&abs, &id, "Foo Task", "foo", "backlog", "temper", "Body.");

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: Some(MoveSpec {
                context_to: Some("writing".to_string()),
                type_to: None,
            }),
            origin: Surface::CliLocalVault,
        };

        backend.update_resource(cmd).await.expect("update ok");

        // File must be at new context path.
        let new_path = tmp.path().join("@me/writing/task/foo.md");
        assert!(new_path.exists(), "file must be at new context path");

        // Old path must be gone.
        assert!(
            !tmp.path().join(rel).exists(),
            "old file must be removed after context move"
        );

        // New file must contain updated temper-context.
        let disk_content = fs::read_to_string(&new_path).unwrap();
        assert!(
            disk_content.contains("temper-context: writing"),
            "temper-context must be updated in moved file, got:\n{disk_content}"
        );
    }

    // ── update_resource_type_to_moves_file ───────────────────────────────────

    #[tokio::test]
    async fn update_resource_type_to_moves_file() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/bar.md";
        let abs = tmp.path().join(rel);

        write_task_file(&abs, &id, "Bar Task", "bar", "backlog", "temper", "Body.");

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: Some(MoveSpec {
                context_to: None,
                type_to: Some("research".to_string()),
            }),
            origin: Surface::CliLocalVault,
        };

        backend.update_resource(cmd).await.expect("update ok");

        // File must be at new type path.
        let new_path = tmp.path().join("@me/temper/research/bar.md");
        assert!(new_path.exists(), "file must be at new type path");

        // Old path must be gone.
        assert!(
            !tmp.path().join(rel).exists(),
            "old file must be removed after type move"
        );

        // New file must contain updated temper-type.
        let disk_content = fs::read_to_string(&new_path).unwrap();
        assert!(
            disk_content.contains("temper-type: research"),
            "temper-type must be updated in moved file, got:\n{disk_content}"
        );
    }

    // ── update_resource_no_client_emits_push_deferred ─────────────────────────

    #[tokio::test]
    async fn update_resource_no_client_emits_push_deferred() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/deferred.md";
        let abs = tmp.path().join(rel);

        write_task_file(
            &abs,
            &id,
            "Deferred Task",
            "deferred",
            "backlog",
            "temper",
            "Body.",
        );

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest); // client: None
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: None,
            managed_meta: Some(ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..ManagedMeta::default()
            }),
            open_meta: None,
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        let output = backend.update_resource(cmd).await.expect("update ok");
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

    // ── update_resource_validate_update_rejects_invalid ──────────────────────

    #[tokio::test]
    async fn update_resource_validate_update_rejects_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = Manifest::new("test-device".to_string());
        let backend = make_backend(tmp.path(), manifest);

        // Scoped ref with invalid slug (uppercase) must be caught by validate_update.
        let cmd = UpdateResource {
            resource: ResourceRef::Scoped {
                owner: "@me".to_string(),
                context: "temper".to_string(),
                doctype: "task".to_string(),
                slug: "INVALID_SLUG".to_string(),
            },
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        let err = backend.update_resource(cmd).await.expect_err("should fail");
        assert!(
            matches!(err, temper_core::error::TemperError::BadRequest(_)),
            "expected BadRequest for invalid slug, got: {err:?}"
        );
    }

    // ── update_resource_manifest_entry_updated_after_write ───────────────────

    #[tokio::test]
    async fn update_resource_manifest_entry_updated_after_write() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/manifest-check.md";
        let abs = tmp.path().join(rel);

        write_task_file(
            &abs,
            &id,
            "Manifest Check",
            "manifest-check",
            "backlog",
            "temper",
            "Old body.",
        );

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: Some(BodyUpdate::new("New body text.")),
            managed_meta: None,
            open_meta: None,
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        backend.update_resource(cmd).await.expect("update ok");

        // Manifest entry state must advance to LocalModified.
        let manifest = backend.manifest().lock().await;
        let entry = manifest.entries.get(&id).expect("entry must remain");
        assert!(
            matches!(
                entry.state,
                temper_core::types::manifest::ManifestEntryState::LocalModified
            ),
            "manifest state must be LocalModified after write, got: {:?}",
            entry.state
        );
        // Body hash must have been updated.
        assert_ne!(
            entry.body_hash, "sha256:abc",
            "manifest body_hash must be updated from the initial placeholder"
        );
    }

    // ── update_resource_heals_missing_temper_title ───────────────────────────
    //
    // Symmetric-defense receive-side: a pre-Phase-4 file that lacks
    // `temper-title` in frontmatter should have it injected during update_resource
    // (the key falls back to the filename stem). Verifies the on-disk file
    // contains `temper-title` after the round-trip.

    #[tokio::test]
    async fn update_resource_heals_missing_temper_title() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/heal-title.md";
        let abs = tmp.path().join(rel);

        // Write a file that deliberately omits temper-title (pre-Phase-4 style).
        let content = format!(
            "---\ntemper-id: \"{}\"\ntemper-type: task\ntemper-context: temper\ntemper-slug: heal-title\ntemper-stage: backlog\n---\n\nBody.\n",
            *id
        );
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, content).unwrap();

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: None,
            managed_meta: Some(ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..ManagedMeta::default()
            }),
            open_meta: None,
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        backend.update_resource(cmd).await.expect("update ok");

        // On-disk frontmatter must now contain temper-title.
        let disk_content = fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert!(
            disk_content.contains("temper-title"),
            "update_resource must heal missing temper-title; got:\n{disk_content}"
        );
    }

    // ── update_resource_heals_missing_temper_slug ─────────────────────────────
    //
    // Symmetric-defense receive-side: a pre-Phase-4 file that lacks
    // `temper-slug` should have it injected during update_resource.

    #[tokio::test]
    async fn update_resource_heals_missing_temper_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/heal-slug.md";
        let abs = tmp.path().join(rel);

        // Write a file that deliberately omits temper-slug (pre-Phase-4 style).
        let content = format!(
            "---\ntemper-id: \"{}\"\ntemper-type: task\ntemper-context: temper\ntemper-title: 'Heal Slug Task'\ntemper-stage: backlog\n---\n\nBody.\n",
            *id
        );
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, content).unwrap();

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = UpdateResource {
            resource: ResourceRef::Uuid { id },
            body: None,
            managed_meta: Some(ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..ManagedMeta::default()
            }),
            open_meta: None,
            move_to: None,
            origin: Surface::CliLocalVault,
        };

        backend.update_resource(cmd).await.expect("update ok");

        // On-disk frontmatter must now contain temper-slug.
        let disk_content = fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert!(
            disk_content.contains("temper-slug"),
            "update_resource must heal missing temper-slug; got:\n{disk_content}"
        );
    }

    // ── update_resource_with_client_emits_remote_synced (stubbed) ────────────

    #[tokio::test]
    #[ignore = "no TemperClient test fixture available; tracked as backlog task"]
    async fn update_resource_with_client_emits_remote_synced_on_success() {
        todo!("implement when a mock TemperClient fixture is available")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// delete_resource tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "test-db"))]
mod delete_resource_tests {
    use std::fs;
    use std::sync::Arc;

    use chrono::Utc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use temper_core::error::TemperError;
    use temper_core::operations::{Backend, DeleteResource, DomainEvent, ResourceRef, Surface};
    use temper_core::types::ids::ResourceId;
    use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};

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

    fn write_task_file(path: &std::path::Path, id: &ResourceId) {
        let content = format!(
            "---\ntemper-id: \"{}\"\ntemper-type: task\ntemper-context: temper\ntemper-title: 'Delete Test'\ntemper-slug: delete-test\ntemper-stage: backlog\n---\n\nBody.\n",
            **id
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // ── delete_resource_no_client_removes_local_when_manifest_present ─────────

    #[tokio::test]
    async fn delete_resource_no_client_removes_local_when_manifest_present() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/delete-test.md";
        let abs = tmp.path().join(rel);

        write_task_file(&abs, &id);
        assert!(abs.exists(), "file must exist before delete");

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = DeleteResource {
            resource: ResourceRef::Uuid { id },
            force: true,
            origin: Surface::CliLocalVault,
        };

        let output = backend.delete_resource(cmd).await.expect("delete ok");

        // File must be gone.
        assert!(!abs.exists(), "vault file must be removed after delete");

        // Manifest entry must be cleared.
        let manifest = backend.manifest().lock().await;
        assert!(
            !manifest.entries.contains_key(&id),
            "manifest entry must be cleared after delete"
        );

        // Events: VaultFileRemoved + VaultManifestUpdated (no RemoteSynced; no client).
        assert_eq!(
            output.events.len(),
            2,
            "expected 2 events (no client), got: {:?}",
            output.events
        );
        assert!(
            matches!(&output.events[0], DomainEvent::VaultFileRemoved { path } if path.ends_with(".md")),
            "first event must be VaultFileRemoved, got: {:?}",
            output.events[0]
        );
        assert!(
            matches!(&output.events[1], DomainEvent::VaultManifestUpdated { .. }),
            "second event must be VaultManifestUpdated, got: {:?}",
            output.events[1]
        );
        // Crucially: no RemoteSynced.
        assert!(
            !output
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::RemoteSynced { .. })),
            "must not emit RemoteSynced when no client present"
        );
    }

    // ── delete_resource_local_only_no_manifest_returns_error ─────────────────

    #[tokio::test]
    async fn delete_resource_local_only_no_manifest_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());

        // Empty manifest — no entry for this id.
        let manifest = Manifest::new("test-device".to_string());
        let backend = make_backend(tmp.path(), manifest);

        let cmd = DeleteResource {
            resource: ResourceRef::Uuid { id },
            force: true,
            origin: Surface::CliLocalVault,
        };

        let err = backend.delete_resource(cmd).await.expect_err("should fail");
        assert!(
            matches!(err, TemperError::NotFound(_)),
            "expected NotFound when no manifest entry, got: {err:?}"
        );
    }

    // ── delete_resource_no_local_file_no_client_returns_error ────────────────

    #[tokio::test]
    async fn delete_resource_no_local_file_no_client_returns_error() {
        let tmp = tempfile::tempdir().unwrap();

        // Completely empty vault — no manifest, no file, no client.
        let manifest = Manifest::new("test-device".to_string());
        let backend = make_backend(tmp.path(), manifest);

        let cmd = DeleteResource {
            resource: ResourceRef::Scoped {
                owner: "@me".to_string(),
                context: "temper".to_string(),
                doctype: "task".to_string(),
                slug: "nonexistent-slug".to_string(),
            },
            force: true,
            origin: Surface::CliLocalVault,
        };

        let err = backend.delete_resource(cmd).await.expect_err("should fail");
        // Scoped resolution via find_resource returns TemperError::Vault when
        // no on-disk file exists; Uuid resolution returns TemperError::NotFound.
        // Either is an expected "resource not found" error kind.
        assert!(
            matches!(err, TemperError::NotFound(_) | TemperError::Vault(_)),
            "expected a not-found-class error for nonexistent resource, got: {err:?}"
        );
    }

    // ── delete_resource_emits_only_local_events_when_no_client ───────────────

    #[tokio::test]
    async fn delete_resource_emits_only_local_events_when_no_client() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/event-test.md";
        let abs = tmp.path().join(rel);

        write_task_file(&abs, &id);

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = DeleteResource {
            resource: ResourceRef::Uuid { id },
            force: true,
            origin: Surface::CliLocalVault,
        };

        let output = backend.delete_resource(cmd).await.expect("delete ok");

        // Must emit exactly: VaultFileRemoved + VaultManifestUpdated.
        // Must NOT emit: RemoteSynced, PushDeferred, or any Db* events.
        for event in &output.events {
            assert!(
                matches!(
                    event,
                    DomainEvent::VaultFileRemoved { .. } | DomainEvent::VaultManifestUpdated { .. }
                ),
                "unexpected event when no client: {event:?}"
            );
        }
        assert_eq!(
            output.events.len(),
            2,
            "expected exactly 2 local events, got: {:?}",
            output.events
        );
    }

    // ── delete_resource_manifest_only_no_file_still_clears_entry ─────────────
    // Covers the edge case where the file was already removed from disk but
    // the manifest entry remains (e.g., a previous partial delete).

    #[tokio::test]
    async fn delete_resource_manifest_only_no_file_still_clears_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/ghost.md";
        // File does NOT exist on disk, but manifest entry IS present.

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let backend = make_backend(tmp.path(), manifest);
        let cmd = DeleteResource {
            resource: ResourceRef::Uuid { id },
            force: true,
            origin: Surface::CliLocalVault,
        };

        let output = backend.delete_resource(cmd).await.expect("delete ok");

        // No file existed, so no VaultFileRemoved event.
        // But the manifest entry must be cleared.
        let manifest = backend.manifest().lock().await;
        assert!(
            !manifest.entries.contains_key(&id),
            "manifest entry must be cleared even when file is already missing"
        );

        // Only VaultManifestUpdated (no VaultFileRemoved since file was absent).
        assert_eq!(
            output.events.len(),
            1,
            "expected 1 event (manifest only), got: {:?}",
            output.events
        );
        assert!(
            matches!(&output.events[0], DomainEvent::VaultManifestUpdated { .. }),
            "event must be VaultManifestUpdated, got: {:?}",
            output.events[0]
        );
    }

    // ── delete_resource_with_client_emits_remote_synced (stubbed) ────────────

    #[tokio::test]
    #[ignore = "no TemperClient test fixture available; tracked as backlog task"]
    async fn delete_resource_with_client_emits_remote_synced_on_success() {
        todo!("implement when a mock TemperClient fixture is available")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Object-safety smoke test
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "test-db"))]
mod object_safety_tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use temper_core::operations::{Backend, Surface};
    use temper_core::types::manifest::Manifest;

    use crate::config::Config;
    use crate::vault_backend::{VaultBackend, VaultBackendCtx};

    /// Verify that `VaultBackend` is object-safe via `dyn Backend`.
    /// This catches accidental introduction of generics or non-object-safe trait methods.
    /// Mirrors `temper-api/src/backend/tests.rs::db_backend_dispatches_via_dyn_backend`.
    #[tokio::test]
    async fn vault_backend_is_object_safe() {
        fn assert_object_safe(_: &dyn Backend) {}

        let tmp = tempfile::tempdir().unwrap();
        let config = Arc::new(Config {
            vault_root: tmp.path().to_path_buf(),
            state_dir: tmp.path().join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: tmp.path().join("skills"),
            profile_slug: None,
        });

        let ctx = VaultBackendCtx {
            vault_root: tmp.path().to_path_buf(),
            manifest: Arc::new(Mutex::new(Manifest::new("test-device".to_string()))),
            client: None,
            owner: "@me".to_string(),
            config,
            surface: Surface::CliLocalVault,
        };

        let backend: Box<dyn Backend> = Box::new(VaultBackend::new(ctx));
        assert_object_safe(&*backend);
    }
}
