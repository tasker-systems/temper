#![cfg(feature = "test-db")]

//! End-to-end tests for the `push_one_resource` primitive in
//! `temper_cli::actions::sync`. Scenarios covered:
//!   - `manifest = None` + `PushTarget::Path` + provisional id — the
//!     primitive resolves the id from frontmatter, POSTs to create a new
//!     server-side resource, and rewrites `temper-provisional-id` →
//!     `temper-id` on disk.
//!   - `manifest = Some(&mut ...)` + `PushTarget::Path` + provisional id —
//!     same POST flow, but the primitive also remaps the manifest entry
//!     from the provisional key to the canonical server id, and populates
//!     all nine entry fields (body/managed/open hashes for local + remote,
//!     state, synced_at, mtime_secs).
//!   - `manifest = Some(&mut ...)` + `PushTarget::Id` + canonical id — the
//!     primitive resolves path via the manifest entry, PUTs the edited
//!     body, and updates the entry's nine fields in place.
//!   - `manifest = None` + `PushTarget::Path` + canonical id — the
//!     primitive PUTs the edited body directly with no manifest side
//!     effects.
//!
//! The CLI-level `temper push <id|path>` wrapper is Task 6 and is not tested
//! here.

mod common;

use temper_cli::actions::sync::{push_one_resource, PushTarget};
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
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

/// `PushTarget::Id` happy path: server-side resource already exists, local
/// file has the canonical `temper-id` and an edited body, manifest entry is
/// keyed by the canonical id. The primitive should PUT (not POST), report
/// `PushKind::Modified`, land the edit server-side, and refresh all nine
/// manifest fields.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn push_one_resource_id_target_pushes_existing_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("push-test-3")
        .await
        .expect("context create");

    // Seed a resource server-side with a real PackedChunk so the ingest path
    // doesn't hit the empty-body gotcha from Task 2.
    let seed_body = "# Seed\n\nOriginal body.".to_string();
    let seed_chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: seed_body.clone(),
        content_hash: format!("{:0>64}", "s3"),
        embedding: vec![0.1_f32; 768],
    };
    let seed_payload = IngestPayload {
        title: "Push Id Seed".to_string(),
        origin_uri: "test://e2e/push-id/seed".to_string(),
        context_name: "push-test-3".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&seed_body)),
        slug: "push-id-seed".to_string(),
        content: seed_body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[seed_chunk]).expect("pack chunks")),
    };
    let seeded = app
        .client
        .ingest()
        .create(&seed_payload)
        .await
        .expect("ingest seed failed");
    let resource_id = ResourceId::from(*seeded.id.as_uuid());

    // Write a local file with the canonical temper-id + an edited body.
    let edited_body = "Edited body via PushTarget::Id.";
    let file_path = app.vault_dir.path().join("push-id-target.md");
    std::fs::write(
        &file_path,
        format!(
            "---\n\
             temper-id: \"{id}\"\n\
             temper-context: push-test-3\n\
             temper-type: research\n\
             temper-created: 2026-04-18T00:00:00Z\n\
             temper-owner: '@me'\n\
             title: Push Id Seed\n\
             slug: push-id-seed\n\
             date: 2026-04-18\n\
             ---\n\
             {edited_body}\n",
            id = resource_id.as_uuid(),
        ),
    )
    .expect("write local file");

    // Seed manifest with entry keyed by canonical id.
    let mut manifest = Manifest::new("e2e-test-device".to_string());
    let file_name = file_path.file_name().unwrap().to_str().unwrap().to_string();
    manifest.entries.insert(
        resource_id,
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
            provisional: false,
        },
    );

    let result = push_one_resource(
        &app.client,
        app.vault_dir.path(),
        PushTarget::Id(resource_id),
        Some(&mut manifest),
    )
    .await
    .expect("push_one_resource (Id target)");

    assert_eq!(result.kind, PushKind::Modified);
    assert_eq!(result.resource_id, resource_id, "server id unchanged");

    let entry = manifest
        .entries
        .get(&resource_id)
        .expect("entry still keyed by canonical id");
    assert_eq!(entry.state, ManifestEntryState::Clean);
    assert!(!entry.provisional);
    assert!(!entry.body_hash.is_empty(), "body_hash populated");
    assert_eq!(entry.body_hash, entry.remote_body_hash);
    assert!(!entry.managed_hash.is_empty(), "managed_hash populated");
    assert_eq!(entry.managed_hash, entry.remote_managed_hash);
    assert_eq!(entry.open_hash, entry.remote_open_hash);
    assert!(entry.mtime_secs.is_some());

    // Server content reflects the edit.
    let server_content = app
        .client
        .resources()
        .content(*resource_id.as_uuid())
        .await
        .expect("fetch server content");
    assert!(
        server_content.markdown.contains(edited_body),
        "server markdown must reflect edited body; got:\n{}",
        server_content.markdown
    );
}

/// `PushTarget::Path` with canonical id and no manifest: PUT the edited
/// body directly, return `PushKind::Modified`, and the caller gets no
/// manifest side effects (since none was passed).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn push_one_resource_path_canonical_id_puts_existing_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("push-test-4")
        .await
        .expect("context create");

    let seed_body = "# Seed 4\n\nOriginal body.".to_string();
    let seed_chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: seed_body.clone(),
        content_hash: format!("{:0>64}", "s4"),
        embedding: vec![0.1_f32; 768],
    };
    let seed_payload = IngestPayload {
        title: "Push Path Canonical Seed".to_string(),
        origin_uri: "test://e2e/push-path-canonical/seed".to_string(),
        context_name: "push-test-4".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&seed_body)),
        slug: "push-path-canonical-seed".to_string(),
        content: seed_body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[seed_chunk]).expect("pack chunks")),
    };
    let seeded = app
        .client
        .ingest()
        .create(&seed_payload)
        .await
        .expect("ingest seed failed");
    let resource_id = ResourceId::from(*seeded.id.as_uuid());

    let edited_body = "Edited body via PushTarget::Path canonical.";
    let file_path = app.vault_dir.path().join("push-path-canonical.md");
    std::fs::write(
        &file_path,
        format!(
            "---\n\
             temper-id: \"{id}\"\n\
             temper-context: push-test-4\n\
             temper-type: research\n\
             temper-created: 2026-04-18T00:00:00Z\n\
             temper-owner: '@me'\n\
             title: Push Path Canonical Seed\n\
             slug: push-path-canonical-seed\n\
             date: 2026-04-18\n\
             ---\n\
             {edited_body}\n",
            id = resource_id.as_uuid(),
        ),
    )
    .expect("write local file");

    let result = push_one_resource(
        &app.client,
        app.vault_dir.path(),
        PushTarget::Path(&file_path),
        None,
    )
    .await
    .expect("push_one_resource (Path canonical)");

    assert_eq!(result.kind, PushKind::Modified);
    assert_eq!(
        result.resource_id, resource_id,
        "canonical id from frontmatter unchanged"
    );

    // Server content reflects the edit.
    let server_content = app
        .client
        .resources()
        .content(*resource_id.as_uuid())
        .await
        .expect("fetch server content");
    assert!(
        server_content.markdown.contains(edited_body),
        "server markdown must reflect edited body; got:\n{}",
        server_content.markdown
    );
}

/// Task 5 relaxation: PushTarget::Id with a manifest hint is the authoritative
/// source for (id, provisional). The primitive must accept a file whose
/// frontmatter has NEITHER temper-id NOR temper-provisional-id, as long as
/// the caller passes the id via PushTarget::Id + a manifest entry. This
/// locks in the sync-path behavior (server-seeded resources whose vault
/// file was written without id echo — see graph_build_e2e_test.rs for the
/// motivating fixture).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn push_one_resource_id_target_accepts_file_without_frontmatter_id(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("push-test-5")
        .await
        .expect("context create");

    // Step 1: Seed a resource server-side (canonical id lives only on the server).
    // Use a real PackedChunk so content round-trips (Task 2 lesson).
    let seed_body = "# Seed 5\n\nServer-seeded resource.".to_string();
    let seed_chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: seed_body.clone(),
        content_hash: format!("{:0>64}", "s5"),
        embedding: vec![0.1_f32; 768],
    };
    let seed_payload = IngestPayload {
        title: "Push No-FM-Id Seed".to_string(),
        origin_uri: "test://e2e/push-no-fm-id/seed".to_string(),
        context_name: "push-test-5".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&seed_body)),
        slug: "push-no-fm-id-seed".to_string(),
        content: seed_body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[seed_chunk]).expect("pack chunks")),
    };
    let seeded = app
        .client
        .ingest()
        .create(&seed_payload)
        .await
        .expect("ingest seed failed");
    let resource_id = ResourceId::from(*seeded.id.as_uuid());

    // Step 2: Write a local file at a vault-relative path whose frontmatter has
    // NO temper-id AND NO temper-provisional-id. Include enough frontmatter
    // for the parse to succeed (temper-context, temper-type, temper-created,
    // temper-owner, title, slug, date).
    let edited_body = "Edited body without frontmatter id.";
    let file_path = app.vault_dir.path().join("push-no-fm-id.md");
    std::fs::write(
        &file_path,
        format!(
            "---\n\
             temper-context: push-test-5\n\
             temper-type: research\n\
             temper-created: 2026-04-18T00:00:00Z\n\
             temper-owner: '@me'\n\
             title: Push No-FM-Id Seed\n\
             slug: push-no-fm-id-seed\n\
             date: 2026-04-18\n\
             ---\n\
             {edited_body}\n"
        ),
    )
    .expect("write local file without frontmatter id");

    // Step 3: Seed a manifest entry keyed by the server's canonical id, pointing
    // at the local file, provisional=false.
    let mut manifest = Manifest::new("e2e-test-device".to_string());
    let file_name = file_path.file_name().unwrap().to_str().unwrap().to_string();
    manifest.entries.insert(
        resource_id,
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
            provisional: false,
        },
    );

    // Step 4: Call push_one_resource(&app.client, vault_root, PushTarget::Id(id), Some(&mut manifest)).
    let result = push_one_resource(
        &app.client,
        app.vault_dir.path(),
        PushTarget::Id(resource_id),
        Some(&mut manifest),
    )
    .await
    .expect("push_one_resource (Id target, no frontmatter id)");

    // Step 5: Assert all invariants:

    // - result.kind == PushKind::Modified (manifest says non-provisional → PUT)
    assert_eq!(result.kind, PushKind::Modified);

    // - result.resource_id == the canonical id (unchanged)
    assert_eq!(result.resource_id, resource_id, "server id unchanged");

    // - file on disk has NOT grown a temper-id — the primitive must not
    //   silently rewrite frontmatter when the id came from manifest
    //   (verify by re-reading file and asserting absence of "temper-id:")
    let file_content = std::fs::read_to_string(&file_path).expect("re-read file");
    assert!(
        !file_content.contains("temper-id:"),
        "file must NOT be rewritten with temper-id when manifest is authoritative; got:\n{}",
        file_content
    );

    // - manifest entry at the canonical id: state == Clean, all 9 fields
    //   populated and self-consistent (body_hash == remote_body_hash, etc.)
    let entry = manifest
        .entries
        .get(&resource_id)
        .expect("entry still keyed by canonical id");
    assert_eq!(entry.state, ManifestEntryState::Clean);
    assert!(!entry.provisional);
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

    // - server content reflects the push (fetch via app.client.resources().content()
    //   and confirm the body we wrote is what the server stored)
    let server_content = app
        .client
        .resources()
        .content(*resource_id.as_uuid())
        .await
        .expect("fetch server content");
    assert!(
        server_content.markdown.contains(edited_body),
        "server markdown must reflect edited body; got:\n{}",
        server_content.markdown
    );
}
