//! Test data setup helpers.
//!
//! `clean_and_seed` removes test-created data each run (while preserving the
//! migration-seeded System / Anonymous profiles) and inserts a seed resource
//! owned by the System profile so that visibility tests have stable data.

use sqlx::PgPool;

// Well-known UUIDs from the R2 seed migration.
pub const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
pub const TEMPER_CONTEXT_ID: &str = "00000000-0000-0000-0003-000000000001";
pub const RESEARCH_DOC_TYPE_ID: &str = "00000000-0000-0000-0001-000000000004";

/// Delete all test-generated data, then create a stable seed resource owned
/// by the System profile.
///
/// Preserves the System and Anonymous profiles inserted by migrations.
pub async fn clean_and_seed(pool: &PgPool) {
    // Delete in reverse FK order. Leave kb_doc_types, kb_contexts,
    // and the two seed profiles intact. `kb_deferred_edges` was dropped in
    // the edges-as-projection cutover (migration 20260522100002); slug
    // forward references now live as `relationship_asserted` events.
    sqlx::query("DELETE FROM kb_resource_edges")
        .execute(pool)
        .await
        .expect("clean kb_resource_edges");

    sqlx::query(
        "DELETE FROM kb_events WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean kb_events");

    sqlx::query("DELETE FROM kb_device_sync_state")
        .execute(pool)
        .await
        .expect("clean kb_device_sync_state");

    sqlx::query("DELETE FROM kb_transfers")
        .execute(pool)
        .await
        .expect("clean kb_transfers");

    sqlx::query("DELETE FROM kb_team_invitations")
        .execute(pool)
        .await
        .expect("clean kb_team_invitations");

    sqlx::query("DELETE FROM kb_team_resources")
        .execute(pool)
        .await
        .expect("clean kb_team_resources");

    sqlx::query("DELETE FROM kb_team_members")
        .execute(pool)
        .await
        .expect("clean kb_team_members");

    sqlx::query("DELETE FROM kb_teams")
        .execute(pool)
        .await
        .expect("clean kb_teams");

    // Remove test resources (not the seed ones if we re-run).
    sqlx::query(
        "DELETE FROM kb_resources WHERE owner_profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test resources");

    // Remove test profiles (keep System + Anonymous).
    sqlx::query(
        "DELETE FROM kb_profile_auth_links WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test auth links");

    sqlx::query(
        "DELETE FROM kb_profiles WHERE id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test profiles");

    // Seed one stable research resource owned by System profile.
    // Use upsert to handle concurrent test setup racing on both id and origin_uri.
    sqlx::query(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES (
            '00000000-0000-0000-0099-000000000001',
            $1, $2,
            'test://seed-resource',
            'Seed Research Doc',
            'seed-research-doc',
            $3, $3,
            true, now(), now()
        )
        ON CONFLICT (id) DO UPDATE SET updated = now()
        "#,
    )
    .bind(uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap())
    .bind(uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap())
    .bind(uuid::Uuid::parse_str(SYSTEM_PROFILE_ID).unwrap())
    .execute(pool)
    .await
    .expect("seed resource");
}

/// Create a test profile and return its UUID.
pub async fn create_test_profile(pool: &PgPool, email: &str) -> uuid::Uuid {
    let (profile_id, _context_id) = create_test_profile_with_context(pool, email).await;
    profile_id
}

/// Create a test profile together with a profile-owned 'temper' context.
///
/// Returns `(profile_id, context_id)`. The owned context satisfies the
/// `contexts_visible_to(profile_id)` gate enforced by
/// `context_service::resolve_by_name` inside `ingest_service::ingest`, so
/// tests that POST to `/api/resources` or `/api/ingest` with `context_name =
/// "temper"` will find a visible context.
///
/// The `kb_contexts_owner_name_unique` constraint is per-owner, so each test
/// profile gets its own 'temper' context that does not collide with the
/// system-seeded one or with other test profiles' contexts.
pub async fn create_test_profile_with_context(
    pool: &PgPool,
    email: &str,
) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let sub = format!("test|{profile_id}");
    let slug = email.split('@').next().unwrap_or("test-user");
    let unique_slug = format!("{slug}-{}", &profile_id.to_string()[..8]);
    sqlx::query(
        r#"INSERT INTO kb_profiles (id, display_name, email, slug)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(profile_id)
    .bind(email)
    .bind(email)
    .bind(&unique_slug)
    .execute(pool)
    .await
    .expect("create test profile");

    sqlx::query(
        r#"INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id)
           VALUES ($1, $2, 'test-provider', $3)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(uuid::Uuid::now_v7())
    .bind(profile_id)
    .bind(&sub)
    .execute(pool)
    .await
    .expect("create test auth link");

    // Create a profile-owned 'temper' context so contexts_visible_to(profile_id)
    // returns a row when ingest_service::ingest calls
    // context_service::resolve_by_name with name = "temper".
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
           VALUES ($1, 'temper', 'kb_profiles', $2)"#,
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("create test profile-owned temper context");

    (profile_id, context_id)
}

/// Create a test resource owned by the given profile and return its UUID.
pub async fn create_test_resource(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    title: &str,
    slug: &str,
) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())"#,
    )
    .bind(id)
    .bind(uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap())
    .bind(uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap())
    .bind(format!("test://{slug}"))
    .bind(title)
    .bind(slug)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("create test resource");

    id
}

/// Create a test resource with a manifest row (including open_meta) and return its UUID.
pub async fn create_test_resource_with_manifest(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    title: &str,
    slug: &str,
    open_meta: serde_json::Value,
) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    let context_id = uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap();
    let doc_type_id = uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap();
    let origin_uri = format!("test://{slug}");

    sqlx::query(
        r#"INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())"#,
    )
    .bind(id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(&origin_uri)
    .bind(title)
    .bind(slug)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("create test resource");

    sqlx::query(
        r#"INSERT INTO kb_resource_manifests
            (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
           VALUES ($1, 'test-hash', '{}', $2, 'test-mhash', 'test-ohash', now())"#,
    )
    .bind(id)
    .bind(&open_meta)
    .execute(pool)
    .await
    .expect("create test manifest");

    id
}

/// Insert a directed edge between two resources by synthesizing a
/// `relationship_asserted` event and projecting its edge row — the same
/// path the migration uses for genesis events. `edge_type` is a legacy
/// frontmatter relation label (e.g. `extends`, `depends_on`); the function
/// maps it to `(edge_kind, polarity, label)` via the same 7-variant table
/// `EdgeType::legacy_mapping()` exposes. Weight defaults to 1.0.
pub async fn create_test_edge(
    pool: &PgPool,
    source_id: uuid::Uuid,
    target_id: uuid::Uuid,
    edge_type: &str,
    profile_id: uuid::Uuid,
) -> uuid::Uuid {
    create_test_edge_weighted(pool, source_id, target_id, edge_type, 1.0, profile_id).await
}

/// As [`create_test_edge`], with an explicit weight (for path-decay tests).
pub async fn create_test_edge_weighted(
    pool: &PgPool,
    source_id: uuid::Uuid,
    target_id: uuid::Uuid,
    edge_type: &str,
    weight: f64,
    profile_id: uuid::Uuid,
) -> uuid::Uuid {
    let (edge_kind, polarity) = match edge_type {
        "parent_of" => ("contains", "forward"),
        "depends_on" | "preceded_by" | "derived_from" | "extends" => ("leads_to", "inverse"),
        "relates_to" | "references" => ("near", "forward"),
        other => panic!("unknown legacy edge_type in test fixture: {other}"),
    };

    let event_id = uuid::Uuid::now_v7();
    let payload = serde_json::json!({
        "source_resource_id": source_id,
        "target": { "kind": "resource", "value": target_id },
        "edge_kind": edge_kind,
        "polarity":  polarity,
        "label":     edge_type,
        "weight":    weight,
    });

    sqlx::query(
        r#"INSERT INTO kb_events (
              id, event_type_id, profile_id, device_id, topic_id, scope_id,
              payload, metadata, "references", correlation_id, occurred_at, created
           )
           SELECT $1,
                  (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'),
                  $2,
                  'fixture',
                  '019e3d6f-2300-7000-8000-000000000050',
                  '019e3d6f-2300-7000-8000-000000000010',
                  $3::jsonb,
                  jsonb_build_object('intent', 'fixture'),
                  '[]'::jsonb,
                  $1,
                  now(),
                  now()"#,
    )
    .bind(event_id)
    .bind(profile_id)
    .bind(&payload)
    .execute(pool)
    .await
    .expect("synthesize relationship_asserted event for fixture");

    let edge_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id,
             edge_kind, polarity, label, weight,
             asserted_by_event_id, last_event_id, is_folded)
           VALUES ($1, $2, $3, $4::edge_kind, $5::edge_polarity, $6, $7, $8, $8, false)"#,
    )
    .bind(edge_id)
    .bind(source_id)
    .bind(target_id)
    .bind(edge_kind)
    .bind(polarity)
    .bind(edge_type)
    .bind(weight)
    .bind(event_id)
    .execute(pool)
    .await
    .expect("project edge row for fixture");

    edge_id
}
