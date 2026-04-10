#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_core::types::sync::{
    MergedResource, SyncCompleteRequest, SyncContextEntries, SyncManifestEntry, SyncStatusRequest,
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
        managed_meta: None,
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
        managed_meta: None,
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
        managed_meta: None,
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
        managed_meta: None,
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
        managed_meta: None,
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
        managed_meta: None,
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
        managed_meta: None,
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
        managed_meta: None,
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
        managed_meta: None,
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
