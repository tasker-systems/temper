#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! Task G — the `?meta_only=true` resource list routed through
//! [`read_selector::list_meta_select`].
//!
//! Covers BOTH selector arms:
//! - `Next`: the new `next_impl::list_meta` projects the WS2-scoped substrate
//!   (`readback::enriched_list`) into `ResourceMetaListResponse`. Fixtures seed
//!   `temper_next.*` directly with raw queries (no macros → no `.sqlx` cache
//!   entries for the test target), mirroring `edge_read_next_test`.
//! - `Legacy` (pre-flip default): delegates verbatim to
//!   `resource_service::list_visible_meta` over the legacy `public` schema —
//!   the additive-safety guarantee that the surface is byte-identical to before.
//!
//! The two arms read DIFFERENT schemas (a `temper_next`-direct seed is invisible
//! to the legacy `public` arm by construction), so each arm seeds into its own
//! schema; the parity asserted is that each arm faithfully returns its seeded
//! resource's managed/open tiers.

mod common;

use sqlx::PgPool;
use temper_api::backend::read_selector;
use temper_api::backend::selection::BackendSelection;
use temper_core::types::resource::ResourceListParams;
use uuid::Uuid;

// ── substrate (temper_next) seeding helpers (mirror edge_read_next_test) ──

/// Seed a bare substrate profile inside a `temper_next` search_path transaction
/// so the `sync_personal_team` AFTER-INSERT trigger (unqualified body) lands the
/// personal team in `temper_next.kb_teams`. Returns `(profile_id, personal_team_id)`.
async fn seed_owner(pool: &PgPool, label: &str) -> (Uuid, Uuid) {
    let id = Uuid::now_v7();
    let handle = format!("{label}-{}", &id.simple().to_string()[..8]);

    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .expect("set search_path");
    sqlx::query(
        "INSERT INTO temper_next.kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
    )
    .bind(id)
    .bind(&handle)
    .bind(label)
    .execute(&mut *tx)
    .await
    .expect("seed substrate profile");
    tx.commit().await.expect("commit");

    let team: Uuid = sqlx::query_scalar("SELECT id FROM temper_next.kb_teams WHERE slug = $1")
        .bind(format!("personal-{handle}"))
        .fetch_one(pool)
        .await
        .expect("personal team created by trigger");
    (id, team)
}

/// Insert an owner-scoped context and share it to the owner's personal team so
/// the homed resource is visible through `resources_visible_to`. Returns the
/// context id.
async fn seed_shared_context(pool: &PgPool, owner: Uuid, team: Uuid, slug: &str) -> Uuid {
    let ctx = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO temper_next.kb_contexts (id, owner_table, owner_id, slug, name)
         VALUES ($1, 'kb_profiles', $2, $3, $3)",
    )
    .bind(ctx)
    .bind(owner)
    .bind(slug)
    .execute(pool)
    .await
    .expect("seed context");
    sqlx::query("INSERT INTO temper_next.kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(ctx)
        .bind(team)
        .execute(pool)
        .await
        .expect("share context to personal team");
    ctx
}

/// Seed the minimal event scaffolding (event type + emitter entity + event) the
/// `kb_properties` NOT-NULL FKs require, returning the event id.
async fn seed_event(pool: &PgPool, owner: Uuid) -> Uuid {
    let etype = Uuid::now_v7();
    sqlx::query("INSERT INTO temper_next.kb_event_types (id, name) VALUES ($1, $2)")
        .bind(etype)
        .bind(format!("test-evt-{}", &etype.simple().to_string()[..8]))
        .execute(pool)
        .await
        .expect("seed event type");
    let entity = Uuid::now_v7();
    sqlx::query("INSERT INTO temper_next.kb_entities (id, profile_id, name) VALUES ($1, $2, $3)")
        .bind(entity)
        .bind(owner)
        .bind("test-entity")
        .execute(pool)
        .await
        .expect("seed entity");
    let event = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO temper_next.kb_events (id, event_type_id, emitter_entity_id)
         VALUES ($1, $2, $3)",
    )
    .bind(event)
    .bind(etype)
    .bind(entity)
    .execute(pool)
    .await
    .expect("seed event");
    event
}

/// The content tier for a seeded substrate resource (params struct — keeps the
/// seed helper under the argument-count lint).
struct SeedProps<'a> {
    title: &'a str,
    doc_type: &'a str,
    stage: &'a str,
    open_key: &'a str,
    open_val: &'a str,
}

/// Home a substrate resource into `ctx` with a `doc_type` property (required by
/// the `enriched_list` INNER JOIN), one managed key (`temper-stage`), and one
/// open key. Returns the resource id.
async fn seed_resource_with_props(
    pool: &PgPool,
    owner: Uuid,
    ctx: Uuid,
    event: Uuid,
    props: SeedProps<'_>,
) -> Uuid {
    let SeedProps {
        title,
        doc_type,
        stage,
        open_key,
        open_val,
    } = props;
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO temper_next.kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(title)
        .bind(format!("temper://test/{id}"))
        .execute(pool)
        .await
        .expect("seed resource");
    sqlx::query(
        "INSERT INTO temper_next.kb_resource_homes
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home resource");

    for (key, value) in [
        ("doc_type", doc_type),
        ("temper-stage", stage),
        (open_key, open_val),
    ] {
        sqlx::query(
            "INSERT INTO temper_next.kb_properties
               (owner_table, owner_id, property_key, property_value,
                asserted_by_event_id, last_event_id)
             VALUES ('kb_resources', $1, $2, $3, $4, $4)",
        )
        .bind(id)
        .bind(key)
        .bind(serde_json::Value::String(value.to_string()))
        .bind(event)
        .execute(pool)
        .await
        .expect("seed property");
    }
    id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_meta_select_next_returns_managed_and_open_tiers(pool: PgPool) {
    let (owner, team) = seed_owner(&pool, "owner").await;
    let ctx = seed_shared_context(&pool, owner, team, "meta-ctx").await;
    let event = seed_event(&pool, owner).await;
    let rid = seed_resource_with_props(
        &pool,
        owner,
        ctx,
        event,
        SeedProps {
            title: "Meta Doc",
            doc_type: "task",
            stage: "active",
            open_key: "priority",
            open_val: "high",
        },
    )
    .await;

    let params = ResourceListParams {
        meta_only: Some(true),
        ..Default::default()
    };
    let resp = read_selector::list_meta_select(BackendSelection::Next, &pool, owner, params)
        .await
        .expect("Next list_meta_select");

    let row = resp
        .rows
        .iter()
        .find(|r| Uuid::from(r.resource_id) == rid)
        .expect("seeded resource present in the meta list");

    // Managed tier — the surviving `temper-stage` workflow key.
    let managed = row.managed_meta.as_ref().expect("managed tier present");
    assert_eq!(managed.stage.as_deref(), Some("active"));

    // Open tier — the user-defined key, verbatim.
    let open = row.open_meta.as_ref().expect("open tier present");
    assert_eq!(
        open.get("priority").and_then(|v| v.as_str()),
        Some("high"),
        "open tier carries the user-defined key"
    );

    // The doctype histogram counts the seeded doc_type.
    assert!(
        resp.facets.doc_type.contains_key("task"),
        "facets must count the seeded doctype"
    );

    // Hashes are §7-dissolved under the Next arm (non-invariants).
    assert!(row.managed_hash.is_empty());
    assert!(row.open_hash.is_empty());
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_meta_select_legacy_delegates_to_list_visible_meta(pool: PgPool) {
    // Additive safety: the Legacy arm IS `list_visible_meta` over `public`.
    let email = format!("g-legacy-{}@example.com", Uuid::new_v4());
    let profile = common::fixtures::create_test_profile(&pool, &email).await;
    let slug = format!("legacy-meta-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let rid = common::fixtures::create_test_resource_with_manifest(
        &pool,
        profile,
        "Legacy Meta Doc",
        &slug,
        serde_json::json!({ "priority": "high" }),
    )
    .await;

    let params = ResourceListParams {
        meta_only: Some(true),
        ..Default::default()
    };
    let resp =
        read_selector::list_meta_select(BackendSelection::Legacy, &pool, profile, params.clone())
            .await
            .expect("Legacy list_meta_select");

    let row = resp
        .rows
        .iter()
        .find(|r| Uuid::from(r.resource_id) == rid)
        .expect("seeded legacy resource present in the meta list");
    assert_eq!(
        row.open_meta
            .as_ref()
            .and_then(|o| o.get("priority"))
            .and_then(|v| v.as_str()),
        Some("high")
    );

    // Pass-through identity: the Legacy arm returns exactly what calling
    // `list_visible_meta` directly returns (behavior unchanged by the routing).
    let direct = temper_api::services::resource_service::list_visible_meta(&pool, profile, params)
        .await
        .expect("direct list_visible_meta");
    assert_eq!(resp.total, direct.total);
    assert_eq!(resp.rows.len(), direct.rows.len());
}
