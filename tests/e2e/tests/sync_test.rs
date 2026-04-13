#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_core::types::managed_meta::MetaUpdatePayload;
use temper_core::types::sync::{
    MergedResource, SyncCompleteRequest, SyncContextEntries, SyncItemKind, SyncManifestEntry,
    SyncStatusRequest,
};
use temper_core::types::Manifest;
use temper_core::vault::Vault;

/// POST /api/sync/status — empty manifest returns empty diff.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_empty_manifest(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let resp = app
        .client
        .sync()
        .status(&SyncStatusRequest { contexts: vec![] })
        .await
        .expect("sync status failed");

    assert!(resp.to_push.is_empty());
    assert!(resp.to_pull.is_empty());
    assert!(resp.conflicts.is_empty());
    assert!(resp.removed.is_empty());
}

/// POST /api/sync/status — server-only resource appears as to_pull.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_detects_server_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Create a context and ingest a resource so the server has something.
    app.client
        .contexts()
        .create("sync-test")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Sync Test Doc".to_string(),
        origin_uri: "test://e2e/sync-status".to_string(),
        context_name: "sync-test".to_string(),
        doc_type_name: "research".to_string(),

        content_hash: Some(
            "synctest00000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "sync-test-doc".to_string(),

        content: "# Sync Test\n\nContent for sync testing.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Send an empty manifest for the context — server should tell us to pull.
    let resp = app
        .client
        .sync()
        .status(&SyncStatusRequest {
            contexts: vec![SyncContextEntries {
                name: "sync-test".to_string(),
                entries: vec![],
            }],
        })
        .await
        .expect("sync status failed");

    assert!(
        !resp.to_pull.is_empty(),
        "expected server-only resource in to_pull, got: {resp:?}"
    );
    // URIs are kb://context/doc_type/uuid format.
    assert!(
        resp.to_pull.iter().any(|p| p.uri.contains("sync-test")),
        "expected sync-test context in to_pull URIs, got: {:?}",
        resp.to_pull
    );
}

/// POST /api/sync/status — matching hash means nothing to sync.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_matching_hash_no_diff(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("sync-match")
        .await
        .expect("context create failed");

    let content_hash =
        "matchtest0000000000000000000000000000000000000000000000000000000".to_string();

    let payload = IngestPayload {
        title: "Matching Hash Doc".to_string(),
        origin_uri: "test://e2e/sync-match".to_string(),
        context_name: "sync-match".to_string(),
        doc_type_name: "research".to_string(),

        content_hash: Some(content_hash.clone()),
        slug: "sync-match-doc".to_string(),

        content: "# Match\n\nSame on both sides.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Build the owner-scoped kb:// URI the CLI emits via Vault::canonical_uri.
    let kb_uri = format!("kb://@{}/sync-match/research/{}", profile.slug, resource.id);

    // Client manifest matches server — no diff expected.
    let resp = app
        .client
        .sync()
        .status(&SyncStatusRequest {
            contexts: vec![SyncContextEntries {
                name: "sync-match".to_string(),
                entries: vec![SyncManifestEntry {
                    uri: kb_uri.clone(),
                    local_hash: content_hash.clone(),
                    remote_hash: content_hash,
                    managed_hash: String::new(),
                    remote_managed_hash: String::new(),
                    open_hash: String::new(),
                    remote_open_hash: String::new(),
                }],
            }],
        })
        .await
        .expect("sync status failed");

    // With matching hashes, nothing should be pushed or pulled for this resource.
    let our_uri = &kb_uri;
    assert!(
        !resp.to_push.iter().any(|p| &p.uri == our_uri),
        "matching hash should not appear in to_push"
    );
    assert!(
        !resp.to_pull.iter().any(|p| &p.uri == our_uri),
        "matching hash should not appear in to_pull"
    );
}

/// POST /api/sync/status with a populated owner-scoped manifest entry.
///
/// The existing `sync_status_matching_hash_no_diff` test at sync_test.rs:103
/// builds URIs manually in the legacy 3-segment format
/// (`kb://sync-match/research/{uuid}`), which happens to work with the
/// current `sync_diff_for_device` SQL function's `split_part(uri, '/', 5)`
/// extraction. That's misleading coverage: the real CLI call path builds
/// owner-scoped URIs (`kb://@<slug>/<ctx>/<type>/<uuid>`) via
/// `Vault::canonical_uri` in `build_status_request`, and `split_part(.., '/',
/// 5)` against those returns the doc_type segment instead of the UUID,
/// blowing up on `::UUID` cast.
///
/// This test drives the real `temper_cli::actions::sync::build_status_request`
/// function to construct the `SyncStatusRequest` from a `Manifest` populated
/// with an owner-scoped entry, then POSTs it via the e2e client. It's the
/// RED for the Bug E fix on `sync_diff_for_device`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_round_trips_owner_scoped_manifest_entry(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("sync-owner-scoped")
        .await
        .expect("context create failed");

    let content_hash =
        "ownsyn0000000000000000000000000000000000000000000000000000000000".to_string();

    let payload = IngestPayload {
        title: "Owner-Scoped Sync Doc".to_string(),
        origin_uri: "test://e2e/sync-owner-scoped".to_string(),
        context_name: "sync-owner-scoped".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(content_hash.clone()),
        slug: "sync-owner-scoped-doc".to_string(),
        content: "# Owner scoped\n\nBuilt via build_status_request.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Build a Manifest with a single owner-scoped entry, mirroring what the
    // Phase 2 Vault migration produces for a synced resource.
    let mut manifest = Manifest::new("e2e-test-device".to_string());
    manifest.entries.insert(
        resource.id,
        temper_core::types::ManifestEntry {
            path: format!(
                "@{}/sync-owner-scoped/research/sync-owner-scoped-doc.md",
                profile.slug
            ),
            body_hash: content_hash.clone(),
            remote_body_hash: content_hash.clone(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
            mtime_secs: None,
            last_audit_id: None,
            provisional: false,
        },
    );

    // Have the real CLI helper construct the request — this uses
    // Vault::canonical_uri under the hood, producing the owner-scoped URI
    // shape that the bug was hiding behind legacy-format fixtures.
    let request = temper_cli::actions::sync::build_status_request(&manifest, &[]);

    // Confirm the request is indeed owner-scoped — if this assertion ever
    // fails we know build_status_request regressed and the rest of the
    // test is no longer exercising what it claims to exercise.
    let built_uri = &request
        .contexts
        .iter()
        .flat_map(|c| &c.entries)
        .next()
        .expect("built request must contain the manifest entry")
        .uri;
    assert!(
        built_uri.starts_with("kb://@") || built_uri.starts_with("kb://+"),
        "build_status_request must emit owner-scoped URIs; got: {built_uri}"
    );

    // The actual bug: POSTing this request drives sync_diff_for_device which
    // parses the URI via split_part(..., '/', 5). Against owner-scoped URIs
    // that returns the literal string "research" (the doc_type), and the
    // ::UUID cast errors the query. We're asserting the endpoint succeeds.
    let resp = app
        .client
        .sync()
        .status(&request)
        .await
        .expect("sync status must succeed with owner-scoped manifest entries");

    // With matching hashes, the entry should not appear in any diff bucket.
    let uri_ref = built_uri.as_str();
    assert!(
        !resp.to_push.iter().any(|p| p.uri == uri_ref),
        "matching hash should not appear in to_push"
    );
    assert!(
        !resp.to_pull.iter().any(|p| p.uri == uri_ref),
        "matching hash should not appear in to_pull"
    );
    assert!(
        !resp.conflicts.iter().any(|p| p.uri == uri_ref),
        "matching hash should not appear in conflicts"
    );
}

/// POST /api/sync/complete — finalize with empty merged_resources.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_complete_empty_round(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let resp = app
        .client
        .sync()
        .complete(&SyncCompleteRequest {
            device_id: "e2e-test-device".to_string(),
            merged_resources: vec![],
        })
        .await
        .expect("sync complete failed");

    assert_eq!(resp.updated_count, 0);
    // last_sync_at should be recent (within last 10 seconds).
    let age = chrono::Utc::now() - resp.last_sync_at;
    assert!(
        age.num_seconds() < 10,
        "last_sync_at should be recent, was {age:?} ago"
    );
}

/// POST /api/sync/complete — update content hash for a merged resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_complete_updates_content_hash(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("sync-complete")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Complete Test Doc".to_string(),
        origin_uri: "test://e2e/sync-complete".to_string(),
        context_name: "sync-complete".to_string(),
        doc_type_name: "research".to_string(),

        content_hash: Some(
            "old0000000000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "sync-complete-doc".to_string(),

        content: "# Complete\n\nFor sync complete testing.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    let new_hash = "new0000000000000000000000000000000000000000000000000000000000000".to_string();

    let resp = app
        .client
        .sync()
        .complete(&SyncCompleteRequest {
            device_id: "e2e-test-device".to_string(),
            merged_resources: vec![MergedResource {
                resource_id: resource.id,
                content_hash: new_hash,
            }],
        })
        .await
        .expect("sync complete failed");

    assert_eq!(resp.updated_count, 1);
}

/// GET /api/sync/manifest — empty vault returns empty manifest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_empty(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest failed");

    // New profile with no resources — manifest should be empty.
    // (Seed resource belongs to system profile, not this test user.)
    assert!(
        resp.items.is_empty(),
        "expected empty manifest for new profile, got {} items",
        resp.items.len()
    );
}

/// GET /api/sync/manifest — returns ingested resources with correct metadata,
/// including strictly owner-scoped `kb://@<slug>/<ctx>/<type>/<ident>` URIs.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_returns_resources(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("mfst-res")
        .await
        .expect("context create failed");

    let content_hash =
        "mfstres000000000000000000000000000000000000000000000000000000000".to_string();

    let payload = IngestPayload {
        title: "Manifest Res Doc".to_string(),
        origin_uri: "test://e2e/mfst-res".to_string(),
        context_name: "mfst-res".to_string(),
        doc_type_name: "research".to_string(),

        content_hash: Some(content_hash.clone()),
        slug: "mfst-res-doc".to_string(),

        content: "# Manifest Test\n\nContent for manifest testing.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest failed");

    assert!(
        !resp.items.is_empty(),
        "expected at least one item in manifest"
    );

    let item = resp
        .items
        .iter()
        .find(|i| i.resource_id == resource.id)
        .expect("expected ingested resource in manifest");

    assert_eq!(item.context, "mfst-res");
    assert_eq!(item.doc_type, "research");
    assert_eq!(item.slug, "mfst-res-doc");
    assert_eq!(item.content_hash, content_hash);

    // URI must be strictly owner-scoped: `kb://@<slug>/<ctx>/<type>/<ident>`.
    // This is the format the drop-legacy migration (20260408000001) requires —
    // any legacy no-sigil URIs would be rejected by resource_for_uri() on the
    // server, so the manifest response MUST emit the owner-scoped form.
    let expected_uri = format!("kb://@{}/mfst-res/research/mfst-res-doc", profile.slug);
    assert_eq!(
        item.uri, expected_uri,
        "sync manifest URI must be owner-scoped (kb://@<slug>/<ctx>/<type>/<ident>); \
         legacy no-sigil URIs are rejected by resource_for_uri() after the \
         drop-legacy migration"
    );

    // Defensive: round-trip the URI through Vault::parse_uri to confirm the
    // shared client-side parser also accepts it.
    let parsed = Vault::parse_uri(&item.uri).expect("emitted URI must parse via Vault::parse_uri");
    assert_eq!(parsed.owner, format!("@{}", profile.slug));
    assert_eq!(parsed.context, "mfst-res");
    assert_eq!(parsed.doc_type, "research");
    assert_eq!(parsed.ident, "mfst-res-doc");
}

/// GET /api/sync/manifest — team-context resources emit the `kb://+<team-slug>/...`
/// form of the owner-scoped URI. This exercises the `kb_teams` branch of
/// `kb_resource_uri()`'s `CASE` which the profile-only test at
/// `sync_manifest_returns_resources` does not reach.
///
/// Coverage for the team branch is important because C+D both slipped past
/// Phase 2 verification; this test pins the shape of team-scoped URI
/// emission so a future regression that e.g. dropped the `kb_teams` LEFT JOIN
/// in `kb_resource_uri()` would fail loudly here.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_emits_team_scoped_uri_for_team_context(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Auto-provision the e2e profile so we know its id.
    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Insert a team, add the authenticated profile as owner member, and
    // create a team-owned context containing a resource the profile owns.
    let team_id = uuid::Uuid::now_v7();
    let team_slug = "mfst-team";
    sqlx::query(
        "INSERT INTO kb_teams
            (id, name, slug, description, is_active, created_by_profile_id, created, updated)
         VALUES ($1, $2, $3, 'Team manifest test', true, $4, now(), now())",
    )
    .bind(team_id)
    .bind(team_slug)
    .bind(team_slug)
    .bind(profile.id)
    .execute(&pool)
    .await
    .expect("insert team");

    sqlx::query(
        "INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at)
         VALUES ($1, $2, $3, 'owner', now())",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(team_id)
    .bind(profile.id)
    .execute(&pool)
    .await
    .expect("add team owner");

    let ctx_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id, created, updated)
         VALUES ($1, 'team-vault', 'kb_teams', $2, now(), now())",
    )
    .bind(ctx_id)
    .bind(team_id)
    .execute(&pool)
    .await
    .expect("insert team context");

    let doc_type_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = $1")
        .bind("research")
        .fetch_one(&pool)
        .await
        .expect("lookup research doc_type_id");

    let resource_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, title, slug, origin_uri,
             originator_profile_id, owner_profile_id, is_active, created, updated)
         VALUES ($1, $2, $3, 'Team Doc', 'team-doc', $4, $5, $5, true, now(), now())",
    )
    .bind(resource_id)
    .bind(ctx_id)
    .bind(doc_type_id)
    .bind(format!("test://team/{resource_id}"))
    .bind(profile.id)
    .execute(&pool)
    .await
    .expect("insert team-scoped resource");

    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest failed");

    let item = resp
        .items
        .iter()
        .find(|i| i.resource_id == temper_core::types::ResourceId::from(resource_id))
        .expect("expected team-scoped resource in manifest");

    // The `kb_teams` arm of kb_resource_uri()'s CASE must emit `kb://+<slug>/...`.
    let expected_uri = format!("kb://+{team_slug}/team-vault/research/team-doc");
    assert_eq!(
        item.uri, expected_uri,
        "team-scoped manifest URI must be `kb://+<team-slug>/...`"
    );

    let parsed = Vault::parse_uri(&item.uri).expect("team URI must parse via Vault::parse_uri");
    assert_eq!(parsed.owner, format!("+{team_slug}"));
    assert_eq!(parsed.context, "team-vault");
    assert_eq!(parsed.doc_type, "research");
    assert_eq!(parsed.ident, "team-doc");
}

/// GET /api/sync/manifest — resource without audit rows returns null last_audit_id.
///
/// Regression test: the sqlx query_as! macro inferred last_audit_id as non-null
/// from local dev data. In production, resources can exist without audit rows
/// (e.g. migrated data), causing a runtime decode error on column 7.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_handles_null_last_audit_id(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("mfst-null-audit")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "No Audit Doc".to_string(),
        origin_uri: "test://e2e/mfst-null-audit".to_string(),
        context_name: "mfst-null-audit".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "nullaudit0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "no-audit-doc".to_string(),
        content: "# No Audit\n\nResource with audit rows removed.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Delete audit rows to simulate a resource without audit trail
    sqlx::query("DELETE FROM kb_resource_audits WHERE resource_id = $1")
        .bind(resource.id)
        .execute(&pool)
        .await
        .expect("delete audit rows");

    // Manifest should still work — last_audit_id will be NULL
    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest should handle null last_audit_id");

    let item = resp
        .items
        .iter()
        .find(|i| i.resource_id == resource.id)
        .expect("resource should appear in manifest");

    assert!(
        item.last_audit_id.is_none(),
        "last_audit_id should be None after removing audit rows"
    );
}

/// GET /api/sync/manifest — inactive resources are excluded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_excludes_inactive(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("manifest-inactive")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Will Be Deleted".to_string(),
        origin_uri: "test://e2e/sync-manifest-inactive".to_string(),
        context_name: "manifest-inactive".to_string(),
        doc_type_name: "research".to_string(),

        content_hash: Some(
            "inactive00000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "will-be-deleted".to_string(),

        content: "# Will Be Deleted".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Delete the resource (soft delete — sets is_active=false)
    app.client
        .resources()
        .delete(resource.id.into())
        .await
        .expect("delete failed");

    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest failed");

    assert!(
        !resp.items.iter().any(|i| i.resource_id == resource.id),
        "deleted resource should not appear in manifest"
    );
}

// ---------------------------------------------------------------------------
// CLI sync_refresh / sync_reset path-construction coverage
//
// Phase 2 verification missed two sites in crates/temper-cli/src/actions/sync.rs
// (one in sync_refresh, one in sync_reset) where ManifestEntry.path was built
// from server items using the 3-segment legacy format
// `{context}/{doc_type}/{slug}.md` instead of the 4-segment owner-scoped form
// `{owner}/{context}/{doc_type}/{slug}.md`. After the Phase 2 CLI Vault
// migration, Vault::parse_rel requires 4 segments, so the latent bug would
// fire on any sync that pulled a server-only resource.
//
// These tests drive the fix via the real sync actions against a live e2e
// server.
// ---------------------------------------------------------------------------

/// sync_refresh against a server with a pulled-only resource must produce
/// a manifest entry whose path is owner-scoped and parses via
/// Vault::parse_rel.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_refresh_produces_owner_scoped_path_for_server_only_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("refresh-path-ctx")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Server Only Doc".to_string(),
        origin_uri: "test://e2e/refresh-path".to_string(),
        context_name: "refresh-path-ctx".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "ref00000000000000000000000000000000000000000000000000000000000a".to_string(),
        ),
        slug: "server-only-doc".to_string(),
        content: "# Server only\n\nnothing local yet.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Empty local manifest — sync_refresh should treat the server resource as
    // new-from-server and build a Pending entry for it.
    let mut manifest = Manifest::new("e2e-test-device".to_string());

    temper_cli::actions::sync::sync_refresh(&app.client, &mut manifest, app.vault_dir.path())
        .await
        .expect("sync_refresh failed");

    let entry = manifest
        .entries
        .get(&resource.id)
        .expect("manifest must contain the pulled resource");

    // The core assertion: the path MUST parse via Vault::parse_rel (4 segments,
    // owner sigil required). This is the shape doctor, sync status, and every
    // other consumer of manifest paths now expects.
    let parsed = Vault::parse_rel(&entry.path).unwrap_or_else(|| {
        panic!(
            "ManifestEntry.path must be owner-scoped and parse via Vault::parse_rel, \
             got: {:?}",
            entry.path
        )
    });

    // The owner must match the authenticated profile's slug (derived from
    // config.owner_for_context or extracted from the server URI — either way,
    // it must be the same owner the server used for kb_resource_uri()).
    let expected_owner = format!("@{}", profile.slug);
    assert_eq!(parsed.owner, expected_owner);
    assert_eq!(parsed.context, "refresh-path-ctx");
    assert_eq!(parsed.doc_type, "research");
    assert_eq!(parsed.slug, "server-only-doc");

    // And the full path should be the canonical rel_path form.
    assert_eq!(
        entry.path,
        format!(
            "{}/refresh-path-ctx/research/server-only-doc.md",
            expected_owner
        )
    );
}

/// sync_reset against a server with an unmatched resource must produce a
/// manifest entry whose path is owner-scoped and parses via Vault::parse_rel.
///
/// sync_reset walks the vault looking for local matches; when there are none,
/// it falls through to the "unmatched server resources" loop and builds a
/// Pending entry from the server item's fields. That construction site is the
/// sibling of the bug sync_refresh had.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_reset_produces_owner_scoped_path_for_server_only_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("reset-path-ctx")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Reset Server Only".to_string(),
        origin_uri: "test://e2e/reset-path".to_string(),
        context_name: "reset-path-ctx".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "rst00000000000000000000000000000000000000000000000000000000000b".to_string(),
        ),
        slug: "reset-server-only".to_string(),
        content: "# Reset\n\nno local copy.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Empty local manifest — sync_reset walks the (empty) vault, finds no
    // local files, and falls through to the unmatched-remote loop which
    // builds a Pending entry from the server item.
    let old_manifest = Manifest::new("e2e-test-device".to_string());

    let (new_manifest, _result) =
        temper_cli::actions::sync::sync_reset(&app.client, &old_manifest, app.vault_dir.path())
            .await
            .expect("sync_reset failed");

    let entry = new_manifest
        .entries
        .get(&resource.id)
        .expect("new manifest must contain the server-only resource");

    let parsed = Vault::parse_rel(&entry.path).unwrap_or_else(|| {
        panic!(
            "sync_reset ManifestEntry.path must parse via Vault::parse_rel, got: {:?}",
            entry.path
        )
    });

    let expected_owner = format!("@{}", profile.slug);
    assert_eq!(parsed.owner, expected_owner);
    assert_eq!(parsed.context, "reset-path-ctx");
    assert_eq!(parsed.doc_type, "research");
    assert_eq!(parsed.slug, "reset-server-only");

    assert_eq!(
        entry.path,
        format!(
            "{}/reset-path-ctx/research/reset-server-only.md",
            expected_owner
        )
    );
}

// ---------------------------------------------------------------------------
// Phase E2 — Layer 2: Sync diff kind discrimination
// ---------------------------------------------------------------------------

/// Drift of the managed_hash on the server while local body + local meta
/// are in sync must produce a `to_pull` item with `kind == MetaOnly`, never
/// a Body pull and never a push.
///
/// Regression anchor for the three-tier diff categorization in the
/// `sync_diff_for_device` SQL function and the `categorize_diff_rows` Rust
/// mapping: the whole point of Phase B/C was to let the server tell the
/// client "only the meta drifted". This test pins that wire shape.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_returns_meta_only_kind_for_meta_drift(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("meta-drift")
        .await
        .expect("context create failed");

    let body_hash = "metadrift0000000000000000000000000000000000000000000000000000000".to_string();
    let initial_payload = IngestPayload {
        title: "Meta Drift Doc".to_string(),
        origin_uri: "test://e2e/meta-drift".to_string(),
        context_name: "meta-drift".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(body_hash.clone()),
        slug: "meta-drift-doc".to_string(),
        content: "# Meta Drift\n\nBody stays, meta moves.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[]).expect("pack chunks")),
    };
    let resource = app
        .client
        .ingest()
        .create(&initial_payload)
        .await
        .expect("ingest failed");

    // Read the stable meta hashes that the client's "last synced" manifest
    // record would have — we'll treat these as the stale
    // remote_managed_hash / remote_open_hash the client already knows about.
    let (server_body, stale_managed, stale_open): (String, String, String) = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource.id)
    .fetch_one(&pool)
    .await
    .expect("fetch pre-update manifest hashes");

    // Server-side: PUT new meta so the server's managed_hash / open_hash
    // advance, while body_hash stays fixed.
    let new_meta_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta: serde_json::json!({
            "temper-type": "research",
            "title": "Meta Drift Doc New Title",
        }),
        open_meta: serde_json::json!({"tags": ["drift"]}),
        managed_hash: "sha256:server_managed_new".to_string(),
        open_hash: "sha256:server_open_new".to_string(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&new_meta_payload)
        .send()
        .await
        .expect("server meta PUT failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Build a Manifest entry whose body hashes match server (clean) and
    // whose managed/open hashes reflect the PRE-update state — i.e. the
    // client hasn't yet observed the drift.
    let mut manifest = Manifest::new("e2e-test-device".to_string());
    manifest.entries.insert(
        resource.id,
        temper_core::types::ManifestEntry {
            path: format!("@{}/meta-drift/research/meta-drift-doc.md", profile.slug),
            body_hash: server_body.clone(),
            remote_body_hash: server_body.clone(),
            managed_hash: stale_managed.clone(),
            remote_managed_hash: stale_managed.clone(),
            open_hash: stale_open.clone(),
            remote_open_hash: stale_open.clone(),
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
            mtime_secs: None,
            last_audit_id: None,
            provisional: false,
        },
    );

    let request = temper_cli::actions::sync::build_status_request(&manifest, &[]);
    let resp = app
        .client
        .sync()
        .status(&request)
        .await
        .expect("sync status failed");

    let pull_for_r = resp
        .to_pull
        .iter()
        .find(|p| p.resource_id == resource.id)
        .unwrap_or_else(|| {
            panic!("expected a to_pull entry for the meta-drifted resource, got: {resp:?}")
        });
    assert_eq!(
        pull_for_r.kind,
        SyncItemKind::MetaOnly,
        "meta-only drift must be categorized as MetaOnly, not Body"
    );
    assert!(
        !resp
            .to_push
            .iter()
            .any(|p| p.resource_id == Some(resource.id)),
        "meta-only drift must not produce a push entry"
    );
}

/// Body drift must still produce a `to_pull` item with `kind == Body`. This
/// is a regression anchor so the wire-level Body vs MetaOnly discrimination
/// cannot collapse into a single bucket.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_returns_body_kind_for_body_drift(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("body-drift")
        .await
        .expect("context create failed");

    let body_hash = "bodydrift0000000000000000000000000000000000000000000000000000000".to_string();
    let payload = IngestPayload {
        title: "Body Drift Doc".to_string(),
        origin_uri: "test://e2e/body-drift".to_string(),
        context_name: "body-drift".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(body_hash.clone()),
        slug: "body-drift-doc".to_string(),
        content: "# Body Drift\n\nServer version.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[]).expect("pack chunks")),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Read server meta hashes so the stale body is the only drifting field.
    let (server_body, server_managed, server_open): (String, String, String) = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource.id)
    .fetch_one(&pool)
    .await
    .expect("fetch manifest hashes");

    // Client manifest: body hashes stale (client thinks remote = some other
    // value) so the SQL categorizes as `to_pull_body`.
    let stale_body = "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    let mut manifest = Manifest::new("e2e-test-device".to_string());
    manifest.entries.insert(
        resource.id,
        temper_core::types::ManifestEntry {
            path: format!("@{}/body-drift/research/body-drift-doc.md", profile.slug),
            body_hash: stale_body.clone(),
            remote_body_hash: stale_body.clone(),
            managed_hash: server_managed.clone(),
            remote_managed_hash: server_managed.clone(),
            open_hash: server_open.clone(),
            remote_open_hash: server_open.clone(),
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
            mtime_secs: None,
            last_audit_id: None,
            provisional: false,
        },
    );
    // Sanity: confirm we actually have a body drift, not a false alarm.
    assert_ne!(server_body, stale_body);

    let request = temper_cli::actions::sync::build_status_request(&manifest, &[]);
    let resp = app
        .client
        .sync()
        .status(&request)
        .await
        .expect("sync status failed");

    let pull_for_r = resp
        .to_pull
        .iter()
        .find(|p| p.resource_id == resource.id)
        .unwrap_or_else(|| {
            panic!("expected a to_pull entry for the body-drifted resource, got: {resp:?}")
        });
    assert_eq!(
        pull_for_r.kind,
        SyncItemKind::Body,
        "body drift must be categorized as Body, not MetaOnly"
    );
}

// ---------------------------------------------------------------------------
// Phase E2 — Layer 3: CLI sync_orchestration round-trips
//
// These tests drive the full `sync_orchestration` path. Setup pattern:
//
//   1. Ingest resources via the API.
//   2. Start with an empty manifest, run `sync_orchestration` once — that
//      empty-manifest round pulls every visible server resource into the
//      vault and populates the manifest with real hashes.
//   3. Now mutate either the local file or the server.
//   4. Call `sync_orchestration` again to observe the chosen-tier behavior.
//
// Driving the real orchestrator (not individual push/pull functions) ensures
// the rehash→status→push→pull→complete pipeline is covered end-to-end.
// ---------------------------------------------------------------------------

/// Simulate a completed initial pull for a single resource — write the
/// file to `vault/{owner}/{ctx}/{doc_type}/{slug}.md` and insert a Clean
/// manifest entry whose body/managed/open hashes match what the server
/// has. This puts the test in the state a real vault would be in AFTER a
/// successful `temper sync` round, so subsequent `sync_orchestration`
/// calls exercise the modification-detection path.
///
/// We replicate the production logic (`pull_resource_body` and the
/// `rehash_manifest` hash computation) rather than driving the full
/// pull path because `sync_orchestration` cannot bootstrap an empty
/// vault from an empty manifest — `build_status_request` emits no
/// contexts for empty manifests, so the server returns no diff.
///
/// After writing the file, we PUT the locally-derived managed/open meta
/// to the server via the meta update endpoint so the server-side manifest
/// hashes match the on-disk hashes. This closes the gap between
/// `build_frontmatter_from_resource`'s output (which includes
/// `temper-created`, `title`, `slug`, `temper-owner` in the managed tier)
/// and whatever skeletal meta the test ingest originally used.
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

    // Emit the file with the canonical `---\n\n` frontmatter terminator
    // (the same shape production `pull_resource_body` writes). The trailing
    // blank means `strip_frontmatter` on this file will return a body that
    // starts with `\n` — matching what `rebuild_file_with_new_meta`
    // preserves across meta-only pulls.
    let frontmatter = temper_cli::actions::ingest::build_frontmatter_from_resource(
        &resource,
        context,
        doc_type,
        content_response.managed_meta.as_ref(),
        content_response.open_meta.as_ref(),
    );
    let vault_content = format!("{frontmatter}{}", content_response.markdown);

    let rel_path = format!("@{profile_slug}/{context}/{doc_type}/{slug}.md");
    let abs_path = app.vault_dir.path().join(&rel_path);
    std::fs::create_dir_all(abs_path.parent().unwrap()).expect("create parent dirs");
    std::fs::write(&abs_path, &vault_content).expect("write vault file");

    // Compute hashes the same way `rehash_manifest` / `pull_resource_body` do.
    let body = temper_cli::actions::sync::strip_frontmatter(&vault_content);
    let local_body_hash = temper_core::hash::compute_body_hash(body);
    let fm_yaml = temper_cli::vault::parse_frontmatter(&vault_content);
    let (managed_meta_split, open_meta_split) = match fm_yaml.as_ref() {
        Some(fm) => temper_core::hash::split_frontmatter_tiers(fm, doc_type),
        None => (serde_json::json!({}), serde_json::json!({})),
    };
    let (managed_hash, open_hash) =
        temper_core::hash::compute_frontmatter_hashes_from_yaml(fm_yaml.as_ref(), doc_type);

    // Overwrite the server's body_hash directly so it matches the
    // leading-`\n`-prefixed body the vault file produces via
    // `strip_frontmatter`. Production's `pull_resource_body` has the same
    // divergence and self-heals via an initial push-body round; tests
    // shortcut that by pre-aligning the manifest row.
    sqlx::query("UPDATE kb_resource_manifests SET body_hash = $1 WHERE resource_id = $2")
        .bind(&local_body_hash)
        .bind(uuid)
        .execute(&app.pool)
        .await
        .expect("align server body_hash for seed");

    // Push the seeded meta to the server so both sides agree. Without this,
    // the server's managed_hash — computed from the skeletal `managed_meta`
    // that `IngestPayload` carried — won't match the file-derived hash, and
    // the first sync_orchestration round will see a spurious meta drift.
    let seed_payload = MetaUpdatePayload {
        resource_id,
        managed_meta: managed_meta_split,
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
        temper_core::types::ManifestEntry {
            path: rel_path,
            body_hash: local_body_hash.clone(),
            remote_body_hash: local_body_hash,
            managed_hash: managed_hash.clone(),
            open_hash: open_hash.clone(),
            remote_managed_hash: managed_hash,
            remote_open_hash: open_hash,
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
            mtime_secs,
            last_audit_id: None,
            provisional: false,
        },
    );
}

/// C1: editing a vault file's open_meta frontmatter to add a `relates_to`
/// declaration must push as meta-only — chunks and body_hash stay put, the
/// new edge appears in `kb_resource_edges`, and the manifest's remote
/// managed/open hashes advance while remote_body_hash does not.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_run_push_meta_only_round_trip(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("push-meta")
        .await
        .expect("context create failed");

    // R1 — the file we will edit. Ship with a real chunk so we can assert
    // chunk preservation after the meta-only push.
    let r1_body = "# Push Meta\n\nBody to preserve.".to_string();
    let r1_chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: r1_body.clone(),
        content_hash: format!("{:0>64}", "c1"),
        embedding: vec![0.1_f32; 768],
    };
    let r1_payload = IngestPayload {
        title: "Push Meta R1".to_string(),
        origin_uri: "test://e2e/push-meta/r1".to_string(),
        context_name: "push-meta".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&r1_body)),
        slug: "push-meta-r1".to_string(),
        content: r1_body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({"tags": ["initial"]})),
        chunks_packed: Some(pack_chunks(&[r1_chunk]).expect("pack chunks")),
    };
    let r1 = app
        .client
        .ingest()
        .create(&r1_payload)
        .await
        .expect("ingest r1 failed");

    // R2 — target for relates_to. Must have real chunks so the seeded pull
    // reconstructs a non-empty body; otherwise local_body_hash in the
    // manifest seed won't match what the server computed at ingest time.
    let r2_body = "# R2\n\nTarget resource.".to_string();
    let r2_chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: r2_body.clone(),
        content_hash: format!("{:0>64}", "c2"),
        embedding: vec![0.2_f32; 768],
    };
    let r2_payload = IngestPayload {
        title: "Push Meta R2".to_string(),
        origin_uri: "test://e2e/push-meta/r2".to_string(),
        context_name: "push-meta".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&r2_body)),
        slug: "push-meta-r2".to_string(),
        content: r2_body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[r2_chunk]).expect("pack chunks")),
    };
    let r2 = app
        .client
        .ingest()
        .create(&r2_payload)
        .await
        .expect("ingest r2 failed");

    // Simulate a completed initial pull: write files + seed manifest entries.
    let mut manifest = Manifest::new("e2e-test-device".to_string());
    seed_synced_manifest_entry(
        &app,
        &mut manifest,
        r1.id,
        &profile.slug,
        "push-meta",
        "research",
        "push-meta-r1",
    )
    .await;
    seed_synced_manifest_entry(
        &app,
        &mut manifest,
        r2.id,
        &profile.slug,
        "push-meta",
        "research",
        "push-meta-r2",
    )
    .await;

    let r1_entry_path = manifest
        .entries
        .get(&r1.id)
        .expect("r1 must be in manifest after seed")
        .path
        .clone();
    let pre_push_entry = manifest.entries.get(&r1.id).cloned().expect("r1 entry");

    let chunks_before: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT chunk_index, content, content_hash FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
    )
    .bind(r1.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks before push");
    assert_eq!(chunks_before.len(), 1, "expected one chunk pre-push");

    // Edit the local file: insert a relates_to line into the frontmatter
    // block. The insertion point is just before the closing `---\n` of the
    // frontmatter so we do not disturb the body.
    let file_abs = app.vault_dir.path().join(&r1_entry_path);
    let original = std::fs::read_to_string(&file_abs).expect("read r1 file");
    let close_idx = original
        .match_indices("\n---\n")
        .next()
        .map(|(i, _)| i + 1)
        .expect("frontmatter close marker present");
    let insertion = format!("relates_to: [\"{}\"]\n", r2.id);
    let mut edited = String::with_capacity(original.len() + insertion.len());
    edited.push_str(&original[..close_idx]);
    edited.push_str(&insertion);
    edited.push_str(&original[close_idx..]);
    std::fs::write(&file_abs, &edited).expect("write edited r1 file");
    // Force rehash of R1 on the next sync — filesystem mtime has 1-second
    // granularity on many platforms, so a seed + edit within the same second
    // would leave `mtime_secs` identical and rehash_manifest would skip the
    // file entirely.
    if let Some(entry) = manifest.entries.get_mut(&r1.id) {
        entry.mtime_secs = None;
    }

    // Now run the orchestrator. The rehash phase will see the managed/open
    // hashes drift, status will classify as to_push_meta, and the push path
    // will call /api/resources/{id}/meta under the hood.
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
    .expect("meta-only push sync_orchestration failed");
    assert_eq!(
        result.error_count,
        0,
        "sync_orchestration must not report any errors (events: {:?})",
        progress.events()
    );
    assert!(
        result.push_count > 0,
        "expected at least one push, got push_count={}, events={:?}",
        result.push_count,
        progress.events()
    );

    // Chunk count + content bytes for R1 must be unchanged.
    let chunks_after: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT chunk_index, content, content_hash FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
    )
    .bind(r1.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks after push");
    assert_eq!(
        chunks_after, chunks_before,
        "meta-only push must not touch kb_chunks rows"
    );

    // The relates_to edge must exist in kb_resource_edges.
    let edge_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_resource_edges \
         WHERE source_resource_id = $1 AND target_resource_id = $2 \
           AND edge_type::TEXT = 'relates_to'",
    )
    .bind(uuid::Uuid::from(r1.id))
    .bind(uuid::Uuid::from(r2.id))
    .fetch_one(&pool)
    .await
    .expect("fetch edges");
    assert_eq!(
        edge_count, 1,
        "relates_to edge must be reconciled from the new open_meta"
    );

    // Manifest: body hashes unchanged; remote meta hashes advanced.
    let post_push_entry = manifest.entries.get(&r1.id).expect("r1 still in manifest");
    assert_eq!(
        post_push_entry.body_hash, pre_push_entry.body_hash,
        "local body_hash must NOT change on a meta-only push"
    );
    assert_eq!(
        post_push_entry.remote_body_hash, pre_push_entry.remote_body_hash,
        "remote_body_hash must NOT change on a meta-only push"
    );
    // The edit touched open_meta only (added `relates_to`), so
    // remote_managed_hash stays put and remote_open_hash advances.
    assert_eq!(
        post_push_entry.remote_managed_hash, pre_push_entry.remote_managed_hash,
        "remote_managed_hash must NOT change when only open_meta was edited"
    );
    assert_ne!(
        post_push_entry.remote_open_hash, pre_push_entry.remote_open_hash,
        "remote_open_hash must advance after pushing new open_meta"
    );
}

/// C2: server-side meta update, client-side nothing to say. The sync round
/// must pull the new managed/open meta as a meta-only diff and write it
/// into the file's frontmatter block without touching the body bytes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_run_pull_meta_only_round_trip(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("pull-meta")
        .await
        .expect("context create failed");

    let body = "# Pull Meta\n\nBody should stay.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "pm"),
        embedding: vec![0.1_f32; 768],
    };
    let payload = IngestPayload {
        title: "Pull Meta Doc".to_string(),
        origin_uri: "test://e2e/pull-meta".to_string(),
        context_name: "pull-meta".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "pull-meta-doc".to_string(),
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
        "pull-meta",
        "research",
        "pull-meta-doc",
    )
    .await;

    let pre_pull_entry = manifest.entries.get(&resource.id).cloned().expect("entry");
    let rel_path = pre_pull_entry.path.clone();
    let file_abs = app.vault_dir.path().join(&rel_path);
    let pre_pull_file = std::fs::read_to_string(&file_abs).expect("read pre-pull file");
    let pre_pull_body_bytes =
        temper_cli::actions::sync::strip_frontmatter(&pre_pull_file).to_string();

    // Server-side meta update: bump both managed and open meta.
    let new_meta = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta: serde_json::json!({
            "temper-type": "research",
            "title": "Pull Meta Doc Retitled",
        }),
        open_meta: serde_json::json!({
            "tags": ["server-side"],
        }),
        managed_hash: "sha256:pull_meta_managed_v2".to_string(),
        open_hash: "sha256:pull_meta_open_v2".to_string(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&new_meta)
        .send()
        .await
        .expect("server meta PUT failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Run the sync round — expect a MetaOnly pull.
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
    .expect("sync_orchestration (pull meta) failed");
    assert_eq!(result.error_count, 0, "events={:?}", progress.events());
    assert_eq!(result.pull_count, 1, "expected exactly one pull");

    // Local file body bytes must be byte-identical to the pre-pull body.
    let post_pull_file = std::fs::read_to_string(&file_abs).expect("read post-pull file");
    let post_pull_body_bytes =
        temper_cli::actions::sync::strip_frontmatter(&post_pull_file).to_string();
    assert_eq!(
        post_pull_body_bytes, pre_pull_body_bytes,
        "body bytes must be preserved across a meta-only pull"
    );

    // Frontmatter reflects the new title.
    assert!(
        post_pull_file.contains("title:") && post_pull_file.contains("Pull Meta Doc Retitled"),
        "frontmatter must reflect the new title:\n{post_pull_file}"
    );
    assert!(
        post_pull_file.contains("server-side"),
        "frontmatter must reflect the new open_meta tag:\n{post_pull_file}"
    );

    // Manifest: body hashes unchanged; managed/open advanced and match server.
    let post_pull_entry = manifest
        .entries
        .get(&resource.id)
        .expect("entry still present");
    assert_eq!(
        post_pull_entry.body_hash, pre_pull_entry.body_hash,
        "body_hash must not move on a meta-only pull"
    );
    assert_eq!(
        post_pull_entry.remote_body_hash, pre_pull_entry.remote_body_hash,
        "remote_body_hash must not move on a meta-only pull"
    );
    assert_ne!(
        post_pull_entry.managed_hash, pre_pull_entry.managed_hash,
        "managed_hash must advance after pulling new managed_meta"
    );
    assert_eq!(
        post_pull_entry.managed_hash, post_pull_entry.remote_managed_hash,
        "local and remote managed_hash must re-agree after the pull"
    );
    assert_ne!(
        post_pull_entry.open_hash, pre_pull_entry.open_hash,
        "open_hash must advance after pulling new open_meta"
    );
    assert_eq!(
        post_pull_entry.open_hash, post_pull_entry.remote_open_hash,
        "local and remote open_hash must re-agree after the pull"
    );
}

/// C3: regression anchor for body pushes. Editing the local file body
/// must push as a full body update — `kb_resource_manifests.body_hash`
/// advances and chunk rows are regenerated.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_run_push_body_round_trip(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("push-body")
        .await
        .expect("context create failed");

    let body = "# Push Body\n\nOriginal body content.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "pb"),
        embedding: vec![0.1_f32; 768],
    };
    let payload = IngestPayload {
        title: "Push Body Doc".to_string(),
        origin_uri: "test://e2e/push-body".to_string(),
        context_name: "push-body".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "push-body-doc".to_string(),
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
        "push-body",
        "research",
        "push-body-doc",
    )
    .await;

    let file_abs = app
        .vault_dir
        .path()
        .join(&manifest.entries.get(&resource.id).unwrap().path);

    // Record baseline chunk rows for comparison later.
    let chunks_before: Vec<(i32, String)> = sqlx::query_as(
        "SELECT chunk_index, content FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
    )
    .bind(resource.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks before");
    let pre_push_body_hash: String =
        sqlx::query_scalar("SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1")
            .bind(resource.id)
            .fetch_one(&pool)
            .await
            .expect("fetch body_hash before");

    // Mutate the body on disk.
    let original = std::fs::read_to_string(&file_abs).expect("read file");
    let mutated = format!("{original}\n\nAppended paragraph from the test.\n");
    std::fs::write(&file_abs, &mutated).expect("write mutated file");
    if let Some(entry) = manifest.entries.get_mut(&resource.id) {
        entry.mtime_secs = None;
    }

    // Run sync_orchestration — expect a body push.
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
    .expect("sync_orchestration (push body) failed");
    assert_eq!(result.error_count, 0, "events={:?}", progress.events());
    assert_eq!(result.push_count, 1, "expected one push");

    // Server body_hash must advance.
    let post_push_body_hash: String =
        sqlx::query_scalar("SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1")
            .bind(resource.id)
            .fetch_one(&pool)
            .await
            .expect("fetch body_hash after");
    assert_ne!(
        post_push_body_hash, pre_push_body_hash,
        "server body_hash must advance after a body push"
    );

    // kb_chunks rowset for R must be regenerated (content differs).
    let chunks_after: Vec<(i32, String)> = sqlx::query_as(
        "SELECT chunk_index, content FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
    )
    .bind(resource.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks after");
    assert_ne!(
        chunks_after, chunks_before,
        "chunk rows must be regenerated after a body push"
    );

    // Manifest `remote_body_hash` matches the new server hash.
    let post_push_entry = manifest.entries.get(&resource.id).expect("entry");
    assert_eq!(
        post_push_entry.remote_body_hash, post_push_body_hash,
        "manifest remote_body_hash must match the new server body_hash"
    );
}

/// C4: regression anchor for body pulls. A server-side body update must
/// be observable as `kind = Body` pull, and the local file body must
/// update to match the server.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_run_pull_body_round_trip(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("pull-body")
        .await
        .expect("context create failed");

    let original_body = "# Pull Body\n\nVersion 1 body.".to_string();
    let original_chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: original_body.clone(),
        content_hash: format!("{:0>64}", "p1"),
        embedding: vec![0.1_f32; 768],
    };
    let payload = IngestPayload {
        title: "Pull Body Doc".to_string(),
        origin_uri: "test://e2e/pull-body".to_string(),
        context_name: "pull-body".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&original_body)),
        slug: "pull-body-doc".to_string(),
        content: original_body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[original_chunk]).expect("pack chunks")),
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
        "pull-body",
        "research",
        "pull-body-doc",
    )
    .await;

    let file_abs = app
        .vault_dir
        .path()
        .join(&manifest.entries.get(&resource.id).unwrap().path);

    // Server-side: ingest UPDATE with a new body.
    let new_body = "# Pull Body\n\nVersion 2 body — server updated.".to_string();
    let new_chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: new_body.clone(),
        content_hash: format!("{:0>64}", "p2"),
        embedding: vec![0.2_f32; 768],
    };
    let update_payload = IngestPayload {
        title: "Pull Body Doc".to_string(),
        origin_uri: "test://e2e/pull-body".to_string(),
        context_name: "pull-body".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&new_body)),
        slug: "pull-body-doc".to_string(),
        content: new_body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[new_chunk]).expect("pack chunks")),
    };
    app.client
        .ingest()
        .update(resource.id.into(), &update_payload)
        .await
        .expect("ingest update failed");

    // Run sync_orchestration — expect a body pull.
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
    .expect("sync_orchestration (pull body) failed");
    assert_eq!(result.error_count, 0, "events={:?}", progress.events());
    assert_eq!(result.pull_count, 1, "expected one pull");

    // Local file body now reflects the server's new body.
    let post_pull_file = std::fs::read_to_string(&file_abs).expect("read post-pull file");
    assert!(
        post_pull_file.contains("Version 2 body"),
        "local file must reflect the server-side body update:\n{post_pull_file}"
    );
    assert!(
        !post_pull_file.contains("Version 1 body"),
        "local file must no longer contain the pre-update body:\n{post_pull_file}"
    );

    // Manifest body_hash matches the new server hash.
    let server_body_hash_after: String =
        sqlx::query_scalar("SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1")
            .bind(resource.id)
            .fetch_one(&pool)
            .await
            .expect("fetch body_hash after");
    let post_pull_entry = manifest.entries.get(&resource.id).expect("entry");
    assert_eq!(
        post_pull_entry.remote_body_hash, server_body_hash_after,
        "manifest remote_body_hash must match the new server hash after a body pull"
    );
}

// ---------------------------------------------------------------------------
// Phase E2 — Layer 4: E1b invariants
// ---------------------------------------------------------------------------

/// C5: the relocation guard must refuse to change `temper-context` through
/// `sync_orchestration`. The guard is unit-tested on its own
/// (`pull_meta_only_relocation_guard`), but this test proves it fires
/// through the full sync path rather than only as a direct function call.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_run_relocation_guard_rejects_context_change(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("ctx-a")
        .await
        .expect("ctx-a create failed");
    // ctx-b must also exist on the server so the `temper-context` cascade
    // in the meta service can find a matching context id. The guard should
    // fire BEFORE the cascade, but keeping ctx-b alive means the test
    // failure mode is clearly "guard rejected" rather than "cascade threw
    // because context not found".
    app.client
        .contexts()
        .create("ctx-b")
        .await
        .expect("ctx-b create failed");

    let body = "# Relocate\n\nShould not move.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "rl"),
        embedding: vec![0.1_f32; 768],
    };
    let payload = IngestPayload {
        title: "Relocate Doc".to_string(),
        origin_uri: "test://e2e/relocate".to_string(),
        context_name: "ctx-a".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "relocate-doc".to_string(),
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
        "ctx-a",
        "research",
        "relocate-doc",
    )
    .await;

    let file_abs = app
        .vault_dir
        .path()
        .join(&manifest.entries.get(&resource.id).unwrap().path);

    // Capture original mtime and file bytes so we can assert they don't
    // move after the aborted pull.
    let original_bytes = std::fs::read(&file_abs).expect("read original bytes");

    // Directly write a managed_meta containing `temper-context: ctx-b`
    // into the server's manifest row without going through the meta
    // service — we want the stored meta to declare a relocation while
    // the resource row itself still lives in ctx-a, so the guard fires on
    // the client's pull side rather than on the server-side cascade.
    let rewritten_managed = serde_json::json!({
        "temper-type": "research",
        "temper-context": "ctx-b",
        "title": "Relocate Doc",
    });
    sqlx::query(
        "UPDATE kb_resource_manifests \
         SET managed_meta = $1, managed_hash = $2, updated = now() \
         WHERE resource_id = $3",
    )
    .bind(&rewritten_managed)
    .bind("sha256:relocate_managed_v2")
    .bind(resource.id)
    .execute(&pool)
    .await
    .expect("rewrite managed_meta on server");

    // Run sync_orchestration — the meta-only pull should fail via the
    // relocation guard and be counted as an error.
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
    .expect("sync_orchestration should return Ok (per-item errors are counted, not propagated)");
    assert!(
        result.error_count > 0,
        "relocation guard must surface as a per-item pull error, events={:?}",
        progress.events()
    );

    // Local file: untouched.
    let post_bytes = std::fs::read(&file_abs).expect("read post-attempt bytes");
    assert_eq!(
        post_bytes, original_bytes,
        "local file must be untouched when the relocation guard rejects the pull"
    );

    // Server: the resource's kb_context_id must NOT have changed. The
    // guard runs on the client side of a meta-only pull; we bypassed the
    // server's cascade by writing managed_meta directly, so ctx-a should
    // still own the resource.
    let ctx_a_id = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT id FROM kb_contexts WHERE name = 'ctx-a' AND kb_owner_id = $1",
    )
    .bind(profile.id)
    .fetch_one(&pool)
    .await
    .expect("lookup ctx-a id");
    let resource_ctx_id: uuid::Uuid =
        sqlx::query_scalar("SELECT kb_context_id FROM kb_resources WHERE id = $1")
            .bind(resource.id)
            .fetch_one(&pool)
            .await
            .expect("fetch resource context id");
    assert_eq!(
        resource_ctx_id, ctx_a_id,
        "resource must still live in ctx-a: the guard prevents the relocation"
    );
}

/// C6: the compounding-newline invariant. Repeatedly pulling new meta from
/// the server must leave the local body bytes byte-identical to the
/// original — no drift per iteration, no compounding of the blank-line
/// separator between frontmatter and body. Regression anchor for the E1b
/// fix in `rebuild_file_with_new_meta`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_run_meta_pull_preserves_body_bytes_across_compounding_pulls(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("compound")
        .await
        .expect("context create failed");

    let body = "# Compound\n\nBody bytes must never drift.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "cp"),
        embedding: vec![0.1_f32; 768],
    };
    let payload = IngestPayload {
        title: "Compound Pull".to_string(),
        origin_uri: "test://e2e/compound".to_string(),
        context_name: "compound".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "compound-pull".to_string(),
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
        "compound",
        "research",
        "compound-pull",
    )
    .await;

    let file_abs = app
        .vault_dir
        .path()
        .join(&manifest.entries.get(&resource.id).unwrap().path);

    let original_file = std::fs::read_to_string(&file_abs).expect("read original");
    let original_body_bytes =
        temper_cli::actions::sync::strip_frontmatter(&original_file).to_string();
    let original_body_hash = manifest
        .entries
        .get(&resource.id)
        .expect("entry")
        .body_hash
        .clone();

    // Three iterations of server-side meta mutation + sync pull. If the
    // compounding-newline bug returns, the stripped body after iteration 2
    // would already carry an extra leading `\n`.
    for (i, tag) in ["one", "two", "three"].iter().enumerate() {
        let new_meta = MetaUpdatePayload {
            resource_id: resource.id,
            managed_meta: serde_json::json!({"temper-type": "research"}),
            open_meta: serde_json::json!({"tags": [tag]}),
            managed_hash: format!("sha256:compound_managed_{i}"),
            open_hash: format!("sha256:compound_open_{i}"),
        };
        let resp = app
            .reqwest_client
            .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
            .header("Authorization", format!("Bearer {}", app.token))
            .json(&new_meta)
            .send()
            .await
            .expect("meta PUT failed");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let progress = temper_cli::actions::progress::NoopProgress;
        let skip_paths = std::collections::HashSet::new();
        temper_cli::actions::sync::sync_orchestration(
            &app.client,
            &mut manifest,
            app.vault_dir.path(),
            &[],
            &progress,
            &skip_paths,
        )
        .await
        .expect("sync_orchestration failed");

        let iteration_file = std::fs::read_to_string(&file_abs).expect("read post-iter file");
        let iteration_body =
            temper_cli::actions::sync::strip_frontmatter(&iteration_file).to_string();
        assert_eq!(
            iteration_body, original_body_bytes,
            "iteration {i}: body bytes must be byte-identical to the pre-loop original"
        );
    }

    // After the loop, manifest body_hash is still the pre-loop value.
    let final_entry = manifest.entries.get(&resource.id).expect("entry");
    assert_eq!(
        final_entry.body_hash, original_body_hash,
        "manifest body_hash must still match the pre-loop value after repeated meta-only pulls"
    );
}
