#![cfg(feature = "test-db")]

//! End-to-end tests for the `pull_one_resource` primitive in
//! `temper_cli::actions::sync`. Two branches:
//!   - `manifest = None`  — snapshot written as `{id}.md` under `vault_root`.
//!   - `manifest = Some(&mut ...)` with a tracked entry — write to the
//!     manifest-resolved vault path and update the entry hashes/state.
//!
//! CLI-level behavior — `temper pull <id>` without a manifest writes
//! `{id}.md` to CWD — is guarded by the wrapper in `commands/pull.rs`. The
//! primitive itself writes snapshots to its `vault_root` arg; the wrapper
//! passes CWD in the no-manifest case.
//
// TODO: a second-round sync test would catch regressions where pull leaves
// manifest hashes stale (reported as spurious drift on next sync-status).
// See sync_test.rs for sync-round coverage.

mod common;

use temper_cli::actions::sync::{pull_one_resource, OwnerResolver, PullBranch};
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

    let mut resolver = OwnerResolver::new(&app.client);
    let result = pull_one_resource(
        &app.client,
        app.vault_dir.path(),
        seeded.id,
        None,
        None,
        &mut resolver,
    )
    .await
    .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::Snapshot);
    assert_eq!(result.title, "Pull Snapshot Test");
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

    let mut resolver = OwnerResolver::new(&app.client);
    let result = pull_one_resource(
        &app.client,
        app.vault_dir.path(),
        ResourceId::from(uuid::Uuid::from(seeded.id)),
        Some(&mut manifest),
        None,
        &mut resolver,
    )
    .await
    .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::ManifestTracked);
    assert_eq!(result.title, "Pull Tracked Test");
    assert_eq!(result.path, abs);
    let entry = manifest.entries.get(&seeded.id).unwrap();
    assert!(!entry.body_hash.is_empty(), "body_hash populated post-pull");
    assert_eq!(
        entry.body_hash, entry.remote_body_hash,
        "hashes agree post-pull (no sync-diff context)"
    );
    assert!(
        !entry.managed_hash.is_empty(),
        "managed_hash populated post-pull"
    );
    assert_eq!(
        entry.managed_hash, entry.remote_managed_hash,
        "managed hashes agree post-pull"
    );
    assert_eq!(
        entry.open_hash, entry.remote_open_hash,
        "open hashes agree post-pull"
    );
    assert!(entry.mtime_secs.is_some(), "mtime_secs populated post-pull");
    assert_eq!(entry.state, ManifestEntryState::Clean);
}

/// Locks down the primitive's contract: the caller chooses the write root
/// for snapshots. Passing a root that is NOT the vault dir must land the
/// file under that passed-in root (and NOT under vault_dir). This is what
/// the CLI wrapper relies on when routing manifest-less pulls to CWD —
/// see the module docstring and `crates/temper-cli/src/commands/pull.rs`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_one_resource_snapshot_lands_in_caller_provided_root(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("pull-snapshot-alt-root")
        .await
        .expect("context create");

    let body = "# Snapshot Alt Root\n\nBody.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "c"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "Snapshot Alt Root Test".to_string(),
        origin_uri: "test://pull-snapshot-alt-root".to_string(),
        context_name: "pull-snapshot-alt-root".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "snapshot-alt-root-test".to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-18"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    // Pass a root that is explicitly NOT the vault dir — simulates the CLI
    // wrapper passing CWD when no manifest is loaded.
    let snapshot_dir = tempfile::TempDir::new().expect("create snapshot dir");
    assert_ne!(
        snapshot_dir.path(),
        app.vault_dir.path(),
        "snapshot_dir must differ from vault_dir for this test to be meaningful"
    );

    let mut resolver = OwnerResolver::new(&app.client);
    let result = pull_one_resource(
        &app.client,
        snapshot_dir.path(),
        seeded.id,
        None,
        None,
        &mut resolver,
    )
    .await
    .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::Snapshot);
    let expected_path = snapshot_dir.path().join(format!("{}.md", seeded.id));
    assert_eq!(
        result.path, expected_path,
        "snapshot must be written under the caller-provided root, not vault_dir"
    );
    assert!(
        expected_path.exists(),
        "snapshot file must exist at caller-provided root: {}",
        expected_path.display()
    );

    // Verify nothing landed in the vault dir.
    let vault_candidate = app.vault_dir.path().join(format!("{}.md", seeded.id));
    assert!(
        !vault_candidate.exists(),
        "snapshot must not appear in vault_dir when caller passed a different root"
    );
}

/// Cross-device sync regression: when sync run pulls a `Body` resource
/// whose ID is not in the local manifest, the primitive must reconstruct
/// the canonical `{owner}/{context}/{doc_type}/{slug}.md` layout, write
/// full frontmatter, and insert a manifest entry — so subsequent syncs
/// hit the ManifestTracked branch instead of looping the bug.
///
/// Pre-fix behavior: dumped raw body markdown to `<vault_root>/<uuid>.md`
/// at the vault root with no frontmatter (the seven stranded UUID files
/// observed after a real `temper sync run`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_one_resource_with_manifest_but_untracked_id_writes_canonical_layout(
    pool: sqlx::PgPool,
) {
    let app = common::setup(pool).await;

    // Profile fetched (and dropped) to keep the pre-flight error path in the
    // test footprint — own resources canonicalize to @me/, so we no longer
    // need profile.slug to build the expected path.
    let _profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("first-sync")
        .await
        .expect("context create");

    let body = "# First Sync\n\nBody arrived from the server.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "d"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "First Sync Test".to_string(),
        origin_uri: "test://first-sync".to_string(),
        context_name: "first-sync".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "first-sync-test".to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-05-07"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    // Manifest is loaded but has NO entry for the seeded id — exactly the
    // cross-device case (laptop ingested, desktop syncs for the first time).
    let mut manifest = Manifest::new("e2e-untracked-device".to_string());
    assert!(
        !manifest.entries.contains_key(&seeded.id),
        "precondition: id must not be tracked"
    );

    let mut resolver = OwnerResolver::new(&app.client);
    let result = pull_one_resource(
        &app.client,
        app.vault_dir.path(),
        seeded.id,
        Some(&mut manifest),
        Some(temper_core::hash::compute_body_hash(&body)),
        &mut resolver,
    )
    .await
    .expect("pull_one_resource");

    assert_eq!(
        result.branch,
        PullBranch::NewlyTracked,
        "manifest-Some + untracked id must take the NewlyTracked branch, not Snapshot"
    );

    // Canonical path for own resources: @me/{context}/{doc_type}/{slug}.md.
    // The 2026-05-10 reversal (plan task 12) makes @me canonical for own
    // private work; explicit @<profile.slug>/ is reserved for legacy files
    // from the PR #70/72 window and for cross-user/team paths.
    let expected_rel = "@me/first-sync/research/first-sync-test.md".to_string();
    let expected_abs = app.vault_dir.path().join(&expected_rel);
    assert_eq!(
        result.path, expected_abs,
        "untracked-id own-resource pull must land at @me/..., not <vault_root>/<uuid>.md"
    );

    // The orphan UUID file must NOT have been written.
    let orphan = app.vault_dir.path().join(format!("{}.md", seeded.id));
    assert!(
        !orphan.exists(),
        "untracked-id pull must not produce orphan UUID file at vault root: {}",
        orphan.display()
    );

    // Frontmatter must be reconstructed — file should have a YAML fence and
    // the canonical managed identity keys, not be a raw body dump.
    let on_disk = std::fs::read_to_string(&expected_abs).expect("file written at canonical layout");
    assert!(
        on_disk.starts_with("---\n"),
        "file must start with YAML fence; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains(&format!("temper-id: {}", seeded.id))
            || on_disk.contains(&format!("temper-id: \"{}\"", seeded.id)),
        "frontmatter must include temper-id (quoted or bare); got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("temper-context: first-sync"),
        "frontmatter must include temper-context; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("temper-slug: first-sync-test"),
        "frontmatter must include temper-slug; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("temper-title:"),
        "frontmatter must include temper-title; got:\n{on_disk}"
    );

    // Manifest must now track the resource at the canonical rel_path with
    // populated hashes — so the next sync hits ManifestTracked, not this
    // branch again.
    let entry = manifest
        .entries
        .get(&seeded.id)
        .expect("manifest must now track the previously-untracked id");
    assert_eq!(
        entry.path, expected_rel,
        "manifest entry must record canonical rel_path"
    );
    assert!(
        entry.body_hash.starts_with("sha256:"),
        "manifest entry must have body_hash populated"
    );
    assert!(
        entry.managed_hash.starts_with("sha256:"),
        "manifest entry must have managed_hash populated"
    );
    assert_eq!(
        entry.state,
        ManifestEntryState::Clean,
        "newly-tracked entry must be Clean (in sync with server)"
    );
}

/// Round-trip regression for the ownership-bug-warning fix
/// (docs/superpowers/specs/2026-05-08-ownership-bug-warning-design.md).
///
/// After a NewlyTracked pull, the file's frontmatter must record the
/// canonical `@me` owner sigil for the user's own private work (NOT an
/// explicit `@<profile.slug>` — that direction was reverted on
/// 2026-05-10; see plan task 12). `preflight_ownership_check` must
/// report no mismatches when called with the requester's profile slug.
/// Together these prove the write side (build_frontmatter_from_resource)
/// and the read side (preflight) agree that own resources canonicalize
/// to `@me`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_one_resource_newly_tracked_writes_canonical_owner_and_passes_preflight(
    pool: sqlx::PgPool,
) {
    use temper_cli::actions::sync::preflight_ownership_check;

    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("ownership-test")
        .await
        .expect("context create");

    let body = "# Ownership Round-Trip\n\nBody arrived from the server.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "e"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "Ownership Round-Trip".to_string(),
        origin_uri: "test://ownership".to_string(),
        context_name: "ownership-test".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "ownership-roundtrip".to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-05-08"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    let mut manifest = Manifest::new("e2e-ownership-device".to_string());
    let mut resolver = OwnerResolver::new(&app.client);
    let result = pull_one_resource(
        &app.client,
        app.vault_dir.path(),
        seeded.id,
        Some(&mut manifest),
        Some(temper_core::hash::compute_body_hash(&body)),
        &mut resolver,
    )
    .await
    .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::NewlyTracked);

    // The pulled file must land under @me/, not @<profile.slug>/. Own
    // resources are canonical at @me/; @<other-slug>/ is reserved for
    // other users / team-shared contexts.
    let expected_rel = "@me/ownership-test/research/ownership-roundtrip.md";
    let expected_abs = app.vault_dir.path().join(expected_rel);
    assert_eq!(
        result.path,
        expected_abs,
        "newly-tracked own-resource pull must land at @me/...; got {}",
        result.path.display()
    );

    // Frontmatter must record @me (the canonical local-vault owner for
    // own private work), not the explicit @<profile.slug>.
    let on_disk = std::fs::read_to_string(&result.path).expect("file written");
    assert!(
        on_disk.contains("temper-owner: '@me'")
            || on_disk.contains("temper-owner: \"@me\"")
            || on_disk.contains("temper-owner: @me"),
        "frontmatter must record canonical owner @me; got:\n{on_disk}"
    );
    let explicit_sq = format!("temper-owner: '@{}'", profile.slug);
    let explicit_dq = format!("temper-owner: \"@{}\"", profile.slug);
    assert!(
        !on_disk.contains(&explicit_sq) && !on_disk.contains(&explicit_dq),
        "frontmatter must NOT record explicit @{} for own resource; got:\n{on_disk}",
        profile.slug
    );

    // Preflight must accept the round-trip cleanly.
    let mismatches = preflight_ownership_check(&manifest, app.vault_dir.path(), &profile.slug);
    assert!(
        mismatches.is_empty(),
        "preflight must report no mismatches for the round-tripped resource; got {mismatches:?}"
    );
}
