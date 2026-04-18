#![cfg(feature = "test-db")]

//! End-to-end tests for the `push_one_resource` primitive in
//! `temper_cli::actions::sync`. Two scenarios:
//!   - `manifest = None` + `PushTarget::Path` — the primitive resolves the id
//!     from frontmatter, POSTs (provisional) to create a new server-side
//!     resource, and rewrites `temper-provisional-id` → `temper-id` on disk.
//!   - `manifest = Some(&mut ...)` + `PushTarget::Path` — same POST flow, but
//!     the primitive also remaps the manifest entry from the provisional key
//!     to the canonical server id, and populates all nine entry fields
//!     (body/managed/open hashes for local + remote, state, synced_at,
//!     mtime_secs).
//!
//! The CLI-level `temper push <id|path>` wrapper is Task 6 and is not tested
//! here.

mod common;

use temper_cli::actions::sync::{push_one_resource, PushTarget};
use temper_core::types::{Manifest, ManifestEntry, ManifestEntryState, PushKind, ResourceId};

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn push_one_resource_path_no_manifest_posts_and_rewrites_provisional(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("push-test")
        .await
        .expect("context create");

    let provisional = uuid::Uuid::now_v7();
    let file_path = app.vault_dir.path().join("push-test-seed.md");
    std::fs::write(
        &file_path,
        format!(
            "---\n\
             temper-provisional-id: \"{provisional}\"\n\
             temper-context: push-test\n\
             temper-type: research\n\
             temper-created: 2026-04-18T00:00:00Z\n\
             temper-owner: '@me'\n\
             title: Push Seed\n\
             slug: push-seed\n\
             date: 2026-04-18\n\
             ---\n\
             Body content.\n"
        ),
    )
    .expect("write seed file");

    let result = push_one_resource(
        &app.client,
        app.vault_dir.path(),
        PushTarget::Path(&file_path),
        None,
    )
    .await
    .expect("push_one_resource");

    assert_eq!(result.kind, PushKind::New);
    assert_ne!(*result.resource_id.as_uuid(), provisional);

    let updated = std::fs::read_to_string(&file_path).expect("read updated file");
    assert!(
        !updated.contains("temper-provisional-id"),
        "temper-provisional-id must be gone from the file; got:\n{updated}"
    );
    assert!(
        updated.contains(&format!("temper-id: \"{}\"", result.resource_id.as_uuid()))
            || updated.contains(&format!("temper-id: {}", result.resource_id.as_uuid())),
        "temper-id with server id must be present; got:\n{updated}"
    );

    // Primitive's title source is `title_from_path` (file stem), matching
    // the existing sync body-push path. That's the contract here — this
    // test asserts the ingest POST went through with a non-empty title
    // and the server round-trips the same value we sent.
    let server = app
        .client
        .resources()
        .get(*result.resource_id.as_uuid())
        .await
        .expect("get resource");
    assert_eq!(server.title, "push-test-seed");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn push_one_resource_path_with_manifest_remaps_entry(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("push-test-2")
        .await
        .expect("context create");

    let provisional = uuid::Uuid::now_v7();
    let file_path = app.vault_dir.path().join("push-test-seed-2.md");
    std::fs::write(
        &file_path,
        format!(
            "---\n\
             temper-provisional-id: \"{provisional}\"\n\
             temper-context: push-test-2\n\
             temper-type: research\n\
             temper-created: 2026-04-18T00:00:00Z\n\
             temper-owner: '@me'\n\
             title: Push Seed 2\n\
             slug: push-seed-2\n\
             date: 2026-04-18\n\
             ---\n\
             Body.\n"
        ),
    )
    .expect("write seed file");

    let mut manifest = Manifest::new("e2e-test-device".to_string());
    let file_name = file_path.file_name().unwrap().to_str().unwrap().to_string();
    manifest.entries.insert(
        ResourceId::from(provisional),
        ManifestEntry {
            path: file_name,
            body_hash: String::new(),
            remote_body_hash: String::new(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: chrono::Utc::now(),
            state: ManifestEntryState::LocalModified,
            mtime_secs: None,
            last_audit_id: None,
            provisional: true,
        },
    );

    let result = push_one_resource(
        &app.client,
        app.vault_dir.path(),
        PushTarget::Path(&file_path),
        Some(&mut manifest),
    )
    .await
    .expect("push_one_resource");

    assert_eq!(result.kind, PushKind::New);
    assert!(
        manifest
            .entries
            .get(&ResourceId::from(provisional))
            .is_none(),
        "provisional key must be removed after remap"
    );
    let entry = manifest
        .entries
        .get(&result.resource_id)
        .expect("entry at server id");
    assert_eq!(entry.state, ManifestEntryState::Clean);
    assert!(!entry.provisional, "provisional flag must be cleared");
    assert!(!entry.body_hash.is_empty(), "body_hash populated");
    assert_eq!(
        entry.body_hash, entry.remote_body_hash,
        "remote body hash mirrors local (push-authored)"
    );
    assert!(!entry.managed_hash.is_empty(), "managed_hash populated");
    assert_eq!(
        entry.managed_hash, entry.remote_managed_hash,
        "remote managed hash mirrors local"
    );
    assert_eq!(
        entry.open_hash, entry.remote_open_hash,
        "remote open hash mirrors local"
    );
    assert!(entry.mtime_secs.is_some(), "mtime_secs populated");
}
