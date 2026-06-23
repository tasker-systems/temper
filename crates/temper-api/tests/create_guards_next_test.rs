#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! WS6 collapse (Task F): create-time guards lifted into `NextBackend::create_resource`.
//!
//! Asserts the substrate create path now applies the same defaults / validation / body-hash dedup
//! the legacy `ingest_service::ingest` ran:
//!   (a) a `task` create with no `temper-stage` comes back `temper-stage: backlog` (default applied);
//!   (b) a managed_meta violating the task schema is REJECTED (not silently written);
//!   (c) creating the same body twice returns the FIRST resource id (dedup).
//!
//! Fixtures seed `temper_next.*` directly with raw queries (no macros → no `.sqlx` cache entries for
//! the test target), plus the public `kb_profiles` row `resolve_profile` maps prod→next through. The
//! prod profile id is preserved verbatim as the substrate profile id (the synthesis invariant), so it
//! doubles as the `resources_visible_to` principal the reconstruct/dedup reads gate on.

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::backend::NextBackend;
use temper_core::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_core::types::ids::ProfileId;
use temper_core::types::managed_meta::ManagedMeta;

/// Seed a minimal writable `temper_next` substrate for a single profile and return its id.
///
/// `NextBackend::create_resource` resolves the caller by natural key (`resolve_profile` reads
/// `public.kb_profiles.slug` then `temper_next.kb_profiles.handle`), resolves the `pete@web` emitter
/// entity, and resolves the owner-scoped `temper` context — so all four must be present. The install
/// migration does NOT seed the substrate event-type registry, so we seed it here (the create write
/// fires `resource_created` / `property_set`).
async fn seed_writable_substrate(pool: &PgPool) -> ProfileId {
    let p = Uuid::now_v7();
    let handle = format!("guards-{}", &p.simple().to_string()[..8]);

    // Public profile (resolve_profile maps prod→next by slug==handle). The id is preserved into the
    // substrate below so the prod id doubles as the temper_next visibility principal.
    sqlx::query(
        "INSERT INTO public.kb_profiles (id, display_name, email, slug) VALUES ($1, $2, $3, $4)",
    )
    .bind(p)
    .bind("Guards Tester")
    .bind(format!("{handle}@test.dev"))
    .bind(&handle)
    .execute(pool)
    .await
    .expect("seed public profile");

    // The substrate inserts run inside a `search_path = temper_next` txn: the `kb_profiles` insert
    // fires `sync_personal_team`, whose body references `kb_teams` unqualified (the context_next_test
    // discipline).
    let mut tx = pool.begin().await.expect("begin substrate seed tx");
    sqlx::query("SET LOCAL search_path = temper_next, public")
        .execute(&mut *tx)
        .await
        .expect("set search_path");

    // Ledger event-type registry (install migration leaves it empty; the create write needs it).
    sqlx::query(
        "INSERT INTO kb_event_types (name) VALUES
           ('resource_created'),('resource_updated'),('resource_deleted'),('resource_rehomed'),
           ('relationship_asserted'),('relationship_retracted'),('relationship_retyped'),
           ('relationship_reweighted'),('relationship_folded'),('relationship_decayed'),
           ('relationship_corrected'),('property_asserted'),('property_set'),('property_retracted'),
           ('property_reweighted'),('property_folded'),('block_created'),('block_mutated'),
           ('block_folded'),('block_provenance_corrected'),('grant_created'),('grant_revoked'),
           ('cogmap_seeded'),('region_materialized'),('delegated_launch'),('invocation_closed'),
           ('lens_created')",
    )
    .execute(&mut *tx)
    .await
    .expect("seed event types");

    // Substrate profile: id preserved == prod id, handle == public slug. Fires sync_personal_team.
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)")
        .bind(p)
        .bind(&handle)
        .bind("Guards Tester")
        .execute(&mut *tx)
        .await
        .expect("seed substrate profile");

    // Per-surface emitter `pete@web` (resolve_emitter for Surface::ApiHttp → marker "web").
    sqlx::query("INSERT INTO kb_entities (id, profile_id, name) VALUES ($1, $2, 'pete@web')")
        .bind(Uuid::now_v7())
        .bind(p)
        .execute(&mut *tx)
        .await
        .expect("seed emitter entity");

    // Profile-owned `temper` context (resolve_context: owner-scoped, slug == slugify(name)).
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
         VALUES ($1, 'kb_profiles', $2, 'temper', 'temper')",
    )
    .bind(Uuid::now_v7())
    .bind(p)
    .execute(&mut *tx)
    .await
    .expect("seed temper context");

    tx.commit().await.expect("commit substrate seed");
    ProfileId::from(p)
}

/// Build a `task` create command homed in the seeded `temper` context.
fn task_create(title: &str, slug: &str, body: &str, managed: ManagedMeta) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: "task".to_string(),
        context: "temper".to_string(),
        title: title.to_string(),
        body: Some(BodyUpdate::new(body)),
        managed_meta: managed,
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: Surface::ApiHttp,
    }
}

async fn substrate_resource_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_resources")
        .fetch_one(pool)
        .await
        .expect("count substrate resources")
}

// ─── (a) doc-type default applied ────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn task_create_applies_stage_default(pool: PgPool) {
    let profile = seed_writable_substrate(&pool).await;

    // No temper-stage supplied → the task default `backlog` must be applied at create time.
    let cmd = task_create(
        "Default Stage Task",
        "default-stage-task",
        "Body prose for the default-stage task.",
        ManagedMeta::default(),
    );
    let out = NextBackend::new(pool.clone(), profile)
        .create_resource(cmd)
        .await
        .expect("create should succeed");

    assert_eq!(
        out.value.stage.as_deref(),
        Some("backlog"),
        "task create with no temper-stage must default to backlog"
    );
}

// ─── (b) invalid managed_meta rejected ───────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invalid_managed_meta_is_rejected(pool: PgPool) {
    let profile = seed_writable_substrate(&pool).await;

    // `not-a-real-stage` violates the task temper-stage enum (backlog/in-progress/done/cancelled).
    let managed = ManagedMeta {
        stage: Some("not-a-real-stage".to_string()),
        ..ManagedMeta::default()
    };
    let cmd = task_create(
        "Invalid Task",
        "invalid-task",
        "Body prose for the invalid task.",
        managed,
    );
    let result = NextBackend::new(pool.clone(), profile)
        .create_resource(cmd)
        .await;

    assert!(
        result.is_err(),
        "managed_meta violating the task schema must be rejected, not silently written"
    );
    assert_eq!(
        substrate_resource_count(&pool).await,
        0,
        "no resource row should be written when validation rejects the create"
    );
}

// ─── (c) body-hash dedup ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn duplicate_body_dedups_to_first_id(pool: PgPool) {
    let profile = seed_writable_substrate(&pool).await;
    let body = "Identical body content that must deduplicate on its substrate body_hash.";

    let out1 = NextBackend::new(pool.clone(), profile)
        .create_resource(task_create(
            "First",
            "first-dedup",
            body,
            ManagedMeta::default(),
        ))
        .await
        .expect("first create should succeed");

    let out2 = NextBackend::new(pool.clone(), profile)
        .create_resource(task_create(
            "Second",
            "second-dedup",
            body,
            ManagedMeta::default(),
        ))
        .await
        .expect("second create should dedup, not fail");

    assert_eq!(
        out2.value.id, out1.value.id,
        "second create with an identical body must return the first resource id (dedup)"
    );
    assert_eq!(
        substrate_resource_count(&pool).await,
        1,
        "dedup must not write a second resource row"
    );
}
