#![cfg(feature = "test-db")]

//! End-to-end test: temper graph build → sync → server reconcile
//!
//! Seeds a fixture vault with a goal and three tasks (two linked to the
//! goal via temper-goal, one with a wikilink to another), runs graph
//! build and sync, and asserts the resulting kb_resource_edges rows.
//! Validates both the CLI-side wikilink write-back and the server-side
//! temper-goal → parent_of extraction added in Phase H.

mod common;

use std::path::Path;

use sqlx::PgPool;
use temper_api::MIGRATOR;
use temper_cli::actions::sync::strip_frontmatter;
use temper_core::types::ingest::IngestPayload;
use temper_core::types::Manifest;
use temper_core::types::ManifestEntryState;

fn write_vault_file(
    vault: &Path,
    rel_path: &str,
    frontmatter: impl AsRef<str>,
    body: impl AsRef<str>,
) {
    let path = vault.join(rel_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!("---\n{}\n---\n{}", frontmatter.as_ref(), body.as_ref()),
    )
    .unwrap();
}

fn graph_build_config(vault: &Path, owner: &str) -> temper_cli::config::Config {
    temper_cli::config::Config {
        vault_root: vault.to_path_buf(),
        state_dir: vault.join(".temper"),
        contexts: vec!["temper".to_string()],
        subscriptions: vec![temper_core::types::vault_config::Subscription {
            context: "temper".to_string(),
            owner: Some(owner.to_string()),
            team: None,
            doc_types: None,
            auto_sync: false,
            merge_policy: temper_core::types::config::MergePolicy::Manual,
            local_paths: Vec::new(),
            repos: Vec::new(),
        }],
        skill_output: vault.join(".skill"),
    }
}

/// Seed a manifest entry for an already-ingested resource: fetch from server,
/// write the canonical vault file, align the server body_hash, and insert
/// a Clean manifest entry. This puts the test in the state of a completed
/// initial sync for the resource.
///
/// Uses `profile_slug` for the vault path (e.g. "@e2e") so paths match
/// what the server returns in URIs.
async fn seed_manifest_entry(
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
        .expect("fetch resource for seed");
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

    // Use profile_slug for the vault path (e.g. "@e2e") — NOT resource.owner_handle
    // which would produce @@me/... paths when owner_handle = "@me".
    let rel_path = format!("@{profile_slug}/{context}/{doc_type}/{slug}.md",);
    let abs_path = app.vault_dir.path().join(&rel_path);
    std::fs::create_dir_all(abs_path.parent().unwrap()).expect("create parent dirs");
    fm.write_to(&abs_path).expect("write vault file");

    let local_body_hash = temper_core::hash::compute_body_hash(fm.body());
    let (managed_hash, open_hash) = fm.hashes();

    // Align server body_hash with the vault file so subsequent sync knows
    // the remote is clean after the seed round.
    sqlx::query("UPDATE kb_resource_manifests SET body_hash = $1 WHERE resource_id = $2")
        .bind(&local_body_hash)
        .bind(uuid)
        .execute(&app.pool)
        .await
        .expect("align server body_hash for seed");

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
        temper_core::types::ManifestEntry {
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

#[sqlx::test(migrator = "MIGRATOR")]
async fn graph_build_then_sync_materializes_edges(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    // Step 1: Ensure profile exists (auto-provisioned on first API call)
    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    // Use profile.slug so vault paths match what the server returns in URIs.
    let owner = format!("@{}", profile.slug);

    // Step 2: Create a context on the server so sync knows about it
    app.client
        .contexts()
        .create("temper")
        .await
        .expect("context create failed");

    // Step 3: Create a goal and two tasks via API so they're in the server DB.
    // Provide pre-computed empty `chunks_packed` to bypass the `ingest-pipeline`
    // feature's `prepare_markdown` + `compute_body_hash` path. Without this,
    // workspace builds (which enable `ingest-pipeline` via `temper-cloud`) would
    // compute the same `sha256("")` for all four empty-content resources and
    // `find_by_body_hash` dedup would collapse them into one server row.
    let empty_chunks =
        Some(temper_core::types::ingest::pack_chunks(&[]).expect("pack empty chunks"));
    let goal = app
        .client
        .ingest()
        .create(&IngestPayload {
            title: "my-goal".to_string(),
            origin_uri: "test://e2e/graph-build/my-goal".to_string(),
            context_name: "temper".to_string(),
            doc_type_name: "goal".to_string(),
            content_hash: None,
            slug: "my-goal".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks.clone(),
        })
        .await
        .expect("ingest goal failed");

    let task_a = app
        .client
        .ingest()
        .create(&IngestPayload {
            title: "task-a".to_string(),
            origin_uri: "test://e2e/graph-build/task-a".to_string(),
            context_name: "temper".to_string(),
            doc_type_name: "task".to_string(),
            content_hash: None,
            slug: "task-a".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-goal": "my-goal"})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks.clone(),
        })
        .await
        .expect("ingest task-a failed");

    let task_b = app
        .client
        .ingest()
        .create(&IngestPayload {
            title: "task-b".to_string(),
            origin_uri: "test://e2e/graph-build/task-b".to_string(),
            context_name: "temper".to_string(),
            doc_type_name: "task".to_string(),
            content_hash: None,
            slug: "task-b".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-goal": "my-goal"})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks.clone(),
        })
        .await
        .expect("ingest task-b failed");

    let source = app
        .client
        .ingest()
        .create(&IngestPayload {
            title: "source".to_string(),
            origin_uri: "test://e2e/graph-build/source".to_string(),
            context_name: "temper".to_string(),
            doc_type_name: "task".to_string(),
            content_hash: None,
            slug: "source".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks.clone(),
        })
        .await
        .expect("ingest source failed");

    // Step 4: Seed vault files and manifest entries for goal/task-a/task-b.
    // This uses seed_manifest_entry which writes vault files and aligns the
    // server's body_hash with the file content.
    let mut manifest = Manifest::new("e2e-test-device".to_string());
    seed_manifest_entry(
        &app,
        &mut manifest,
        goal.id,
        &profile.slug,
        "temper",
        "goal",
        "my-goal",
    )
    .await;
    seed_manifest_entry(
        &app,
        &mut manifest,
        task_a.id,
        &profile.slug,
        "temper",
        "task",
        "task-a",
    )
    .await;
    seed_manifest_entry(
        &app,
        &mut manifest,
        task_b.id,
        &profile.slug,
        "temper",
        "task",
        "task-b",
    )
    .await;

    // Write source.md with wikilink content. The path uses owner (profile slug).
    write_vault_file(
        app.vault_dir.path(),
        &format!("{owner}/temper/task/source.md"),
        format!("temper-context: temper\ntemper-type: task\ntemper-owner: '{owner}'\ntemper-title: source\ntemper-slug: source"),
        "See [[task-a]] for the background.\n",
    );

    // Step 5: Run graph build — this scans source.md's body for wikilinks and
    // writes references into its frontmatter.
    let vault_path = app.vault_dir.path();
    let config = graph_build_config(vault_path, &owner);
    let params = temper_cli::actions::graph_build::GraphBuildParams {
        context_filter: None,
        dry_run: false,
        verbose: false,
    };
    let report =
        temper_cli::actions::graph_build::run(&config, params).expect("graph build failed");
    assert_eq!(
        report.files_modified, 1,
        "expected source.md to be modified"
    );
    assert_eq!(
        report.references_added, 1,
        "expected task-a to be added as reference"
    );

    // Step 6: Insert source into the manifest as LocalModified. The remote_*_hash
    // fields must match what the server actually stored in kb_resource_manifests
    // when source was ingested — NOT empty strings. With the `ingest-pipeline`
    // feature enabled (which workspace builds activate via temper-cloud),
    // `create_resource_with_manifest` runs `compute_body_hash(payload.content)`
    // even on empty content, so server.body_hash is `sha256("")`, not `''`.
    // Query the actual server-side row instead of guessing.
    let source_uuid: uuid::Uuid = source.id.into();
    let server_source_hashes: (String, String, String) = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(source_uuid)
    .fetch_one(&app.pool)
    .await
    .expect("fetch source server-side hashes");

    let source_rel_path = format!("{owner}/temper/task/source.md");
    let source_abs_path = app.vault_dir.path().join(&source_rel_path);
    let source_content = std::fs::read_to_string(&source_abs_path).expect("read source.md");
    let source_fm = temper_core::frontmatter::Frontmatter::try_from(source_content.as_str())
        .expect("source.md should have valid frontmatter");
    let (source_managed_hash, source_open_hash) = source_fm.hashes();
    let source_body = strip_frontmatter(&source_content);
    let source_body_hash = temper_core::hash::compute_body_hash(source_body);
    let source_mtime = std::fs::metadata(&source_abs_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs() as i64)
        });

    manifest.entries.insert(
        source.id,
        temper_core::types::ManifestEntry {
            path: source_rel_path,
            body_hash: source_body_hash,
            remote_body_hash: server_source_hashes.0.clone(),
            managed_hash: source_managed_hash,
            open_hash: source_open_hash,
            remote_managed_hash: server_source_hashes.1.clone(),
            remote_open_hash: server_source_hashes.2.clone(),
            synced_at: chrono::Utc::now(),
            state: ManifestEntryState::LocalModified,
            mtime_secs: source_mtime,
            last_audit_id: None,
            provisional: false,
        },
    );

    // Step 7: Sync to push the updated frontmatter references via the full
    // sync_orchestration pipeline (rehash → status → push → server reconcile).
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
        "sync_orchestration had errors: {:?}",
        progress.events()
    );
    assert!(
        result.push_count > 0,
        "expected at least one push, got push_count={}",
        result.push_count
    );

    // ── Assertions ─────────────────────────────────────────────────

    // 1. source → references → task-a edge (from CLI graph build write-back,
    //    pushed via sync and created by server-side reconcile_edges)
    let source_ref_task_a: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'source'
           AND tgt.slug = 'task-a'
           AND e.edge_type::text = 'references'
           AND e.metadata->>'provenance' = 'frontmatter'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        source_ref_task_a, 1,
        "source should reference task-a after graph build + sync"
    );

    // 2. my-goal → parent_of → task-a (from server-side temper-goal extraction)
    let goal_parent_a: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'my-goal'
           AND tgt.slug = 'task-a'
           AND e.edge_type::text = 'parent_of'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        goal_parent_a, 1,
        "my-goal should be parent_of task-a via server-side temper-goal extraction"
    );

    // 3. my-goal → parent_of → task-b
    let goal_parent_b: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'my-goal'
           AND tgt.slug = 'task-b'
           AND e.edge_type::text = 'parent_of'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(goal_parent_b, 1);

    // 4. Verify the vault file does NOT contain a `parent:` field in open_meta —
    // the temper-goal derivation is server-side only, not a CLI write-back.
    let task_a_content = std::fs::read_to_string(
        app.vault_dir
            .path()
            .join(&format!("{owner}/temper/task/task-a.md")),
    )
    .unwrap();
    assert!(
        !task_a_content
            .lines()
            .any(|l| l.trim_start().starts_with("parent:")),
        "task-a.md should not have a parent: field — temper-goal derivation is server-side"
    );
}
