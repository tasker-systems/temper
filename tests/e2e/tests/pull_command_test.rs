#![cfg(feature = "test-db")]

//! End-to-end tests for the `pull_one_resource` primitive in
//! `temper_cli::actions::sync`. Two branches:
//!   - `manifest = None`  — snapshot written as `{id}.md` under `vault_root`.
//!   - `manifest = Some(&mut ...)` with a tracked entry — write to the
//!     manifest-resolved vault path and update the entry hashes/state.

mod common;

use temper_cli::actions::sync::{pull_one_resource, PullBranch};
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_core::types::{Manifest, ManifestEntry, ManifestEntryState, ResourceId};

/// `pull_one_resource` with `manifest = None` writes a snapshot `{id}.md`
/// at `vault_root`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_one_resource_without_manifest_writes_snapshot(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("pull-snapshot")
        .await
        .expect("context create");

    let body = "# Pull Snapshot\n\nSnapshot body.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "a"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "Pull Snapshot Test".to_string(),
        origin_uri: "test://pull-snapshot".to_string(),
        context_name: "pull-snapshot".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "pull-snapshot-test".to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-18"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    let result = pull_one_resource(&app.client, app.vault_dir.path(), seeded.id, None)
        .await
        .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::Snapshot);
    let expected_path = app.vault_dir.path().join(format!("{}.md", seeded.id));
    assert_eq!(result.path, expected_path);
    assert!(
        expected_path.exists(),
        "snapshot file must exist at {}",
        expected_path.display()
    );
    let body = std::fs::read_to_string(&expected_path).unwrap();
    assert!(
        body.contains("Pull Snapshot"),
        "snapshot body must include content: {body}"
    );
}

/// `pull_one_resource` with a tracked manifest entry writes to the
/// manifest-resolved path and updates the entry (body_hash populated,
/// state=Clean).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_one_resource_with_manifest_writes_to_vault_and_updates_entry(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("pull-tracked")
        .await
        .expect("context create");

    let payload = IngestPayload {
        title: "Pull Tracked Test".to_string(),
        origin_uri: "test://pull-tracked".to_string(),
        context_name: "pull-tracked".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some("b".repeat(64)),
        slug: "pull-tracked-test".to_string(),
        content: "# Pull Tracked\n\nTracked body.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-18"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    // Path convention is `@{profile_slug}/{context}/{doc_type}/{slug}.md` —
    // matches what the server returns and what `Vault::parse_rel` expects.
    let rel_path = format!(
        "@{}/pull-tracked/research/pull-tracked-test.md",
        profile.slug
    );
    let abs = app.vault_dir.path().join(&rel_path);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(
        &abs,
        format!("---\ntemper-id: \"{}\"\n---\nstub\n", seeded.id),
    )
    .unwrap();

    let mut manifest = Manifest::new("e2e-test-device".to_string());
    manifest.entries.insert(
        seeded.id,
        ManifestEntry {
            path: rel_path.clone(),
            body_hash: String::new(),
            remote_body_hash: String::new(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: chrono::Utc::now(),
            state: ManifestEntryState::Clean,
            mtime_secs: None,
            last_audit_id: None,
            provisional: false,
        },
    );

    let result = pull_one_resource(
        &app.client,
        app.vault_dir.path(),
        ResourceId::from(uuid::Uuid::from(seeded.id)),
        Some(&mut manifest),
    )
    .await
    .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::ManifestTracked);
    assert_eq!(result.path, abs);
    let entry = manifest.entries.get(&seeded.id).unwrap();
    assert!(!entry.body_hash.is_empty(), "body_hash populated post-pull");
    assert_eq!(entry.state, ManifestEntryState::Clean);
}
