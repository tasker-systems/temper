//! E2E test: `temper sync run` reclassifies a missing-but-tracked file
//! as LocallyMissing and pulls it back from the server, instead of
//! erroring with "vault file missing".
//!
//! Drives the real `sync_orchestration` function (the same entry point
//! `temper sync run` invokes). The harness pattern mirrors the
//! sync_orchestration round-trip tests in `sync_test.rs`: seed a Clean
//! manifest entry, mutate (here: delete the vault file), then run sync
//! and assert push_count == 0 and pull_count == 1.

#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload};
use temper_core::types::{Manifest, ManifestEntry, ManifestEntryState};

/// Inline copy of the seeding helper from `sync_test.rs` — replicates the
/// post-first-sync state for one resource so the subsequent
/// `sync_orchestration` round exercises the modification-detection path.
async fn seed_synced_manifest_entry(
    app: &common::E2eTestApp,
    manifest: &mut Manifest,
    resource_id: temper_core::types::ResourceId,
    profile_slug: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
) {
    let uuid: uuid::Uuid = resource_id.into();
    let resource = app
        .client
        .resources()
        .get(uuid)
        .await
        .expect("fetch resource row for seed");
    let content_response = app
        .client
        .resources()
        .content(uuid)
        .await
        .expect("fetch resource content for seed");

    let managed_value = content_response
        .managed_meta
        .as_ref()
        .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null));
    let fm = temper_cli::actions::ingest::build_frontmatter_from_resource(
        &resource,
        context,
        doc_type,
        &format!("@{profile_slug}"),
        temper_cli::actions::ingest::normalize_body_for_vault(&content_response.markdown),
        managed_value.as_ref(),
        content_response.open_meta.as_ref(),
    )
    .expect("build_frontmatter_from_resource");

    let rel_path = format!("@{profile_slug}/{context}/{doc_type}/{slug}.md");
    let abs_path = app.vault_dir.path().join(&rel_path);
    std::fs::create_dir_all(abs_path.parent().unwrap()).expect("create parent dirs");
    fm.write_to(&abs_path).expect("write vault file");

    let local_body_hash = temper_core::hash::compute_body_hash(fm.body());
    let managed_meta_split = fm.managed_json();
    let open_meta_split = fm.open_json();
    let (managed_hash, open_hash) = fm.hashes();

    sqlx::query("UPDATE kb_resource_manifests SET body_hash = $1 WHERE resource_id = $2")
        .bind(&local_body_hash)
        .bind(uuid)
        .execute(&app.pool)
        .await
        .expect("align server body_hash for seed");

    let managed_meta_typed: ManagedMeta =
        serde_json::from_value(managed_meta_split).expect("managed_meta_split → typed");
    let seed_payload = MetaUpdatePayload {
        resource_id,
        managed_meta: managed_meta_typed,
        open_meta: open_meta_split,
        managed_hash: managed_hash.clone(),
        open_hash: open_hash.clone(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{uuid}/meta")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&seed_payload)
        .send()
        .await
        .expect("seed meta PUT failed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "seed meta PUT returned non-OK: {}",
        resp.status()
    );

    let mtime_secs = std::fs::metadata(&abs_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs() as i64)
        });

    manifest.entries.insert(
        resource_id,
        ManifestEntry {
            path: rel_path,
            body_hash: local_body_hash.clone(),
            remote_body_hash: local_body_hash,
            managed_hash: managed_hash.clone(),
            open_hash: open_hash.clone(),
            remote_managed_hash: managed_hash,
            remote_open_hash: open_hash,
            synced_at: chrono::Utc::now(),
            state: ManifestEntryState::Clean,
            mtime_secs,
            last_audit_id: None,
            provisional: false,
        },
    );
}

/// A manifest entry whose vault file has been removed must be pulled back
/// from the server on the next `sync run`, not pushed (which would fail
/// with "vault file missing").
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_run_pulls_locally_missing_entries(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("locally-missing")
        .await
        .expect("context create failed");

    // 1. Ingest a resource and seed the manifest as if a prior sync had
    //    landed it.
    let body = "# Recoverable\n\nThis file gets removed and recovered.\n".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "lm1"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "Recoverable".to_string(),
        origin_uri: "test://e2e/locally-missing/recoverable".to_string(),
        context_name: "locally-missing".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "recoverable".to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    let mut manifest = Manifest::new("e2e-test-device".to_string());
    seed_synced_manifest_entry(
        &app,
        &mut manifest,
        resource.id,
        &profile.slug,
        "locally-missing",
        "task",
        "recoverable",
    )
    .await;

    let rel_path = manifest.entries.get(&resource.id).unwrap().path.clone();
    let abs_path = app.vault_dir.path().join(&rel_path);
    assert!(
        abs_path.exists(),
        "test setup: file should exist before removal at {}",
        abs_path.display()
    );

    // 2. Remove the local file. The manifest still tracks it.
    std::fs::remove_file(&abs_path).expect("remove vault file");
    assert!(
        !abs_path.exists(),
        "vault file should be gone after manual removal"
    );

    // 3. Run sync_orchestration. With the routing fix, the entry should
    //    be marked LocallyMissing during rehash and pulled back — not
    //    misrouted into the push set (which would error with
    //    "vault file missing").
    let progress = temper_cli::actions::progress::CollectingProgress::new();
    let skip_paths = std::collections::HashSet::new();
    let result = temper_cli::actions::sync::sync_orchestration(
        &app.client,
        &mut manifest,
        app.vault_dir.path(),
        &[],
        &progress,
        &skip_paths,
    )
    .await
    .expect("sync_orchestration failed");

    assert_eq!(
        result.error_count,
        0,
        "sync should not produce errors; events={:?}",
        progress.events()
    );
    assert_eq!(
        result.push_count,
        0,
        "LocallyMissing entries must NOT be pushed; events={:?}",
        progress.events()
    );
    assert_eq!(
        result.pull_count,
        1,
        "LocallyMissing entries must be pulled; events={:?}",
        progress.events()
    );

    // 4. File reappears on disk with the original content.
    assert!(
        abs_path.exists(),
        "file should be restored by sync run at {}",
        abs_path.display()
    );
    let restored = std::fs::read_to_string(&abs_path).expect("read restored file");
    assert!(
        restored.contains("Recoverable"),
        "restored body should match server-side content; got: {restored}"
    );

    // 5. Manifest entry must transition back to Clean after the pull.
    let post_pull_entry = manifest
        .entries
        .get(&resource.id)
        .expect("entry must still exist post-pull");
    assert_eq!(
        post_pull_entry.state,
        ManifestEntryState::Clean,
        "entry state should return to Clean after pull recovery"
    );
}
