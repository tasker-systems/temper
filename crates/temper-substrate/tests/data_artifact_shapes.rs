#![cfg(feature = "artifact-tests")]
//! Data-artifact shape registry — the Beat A substrate (Task 1, plan
//! `internal/superpowers/plans/2026-08-21-data-artifact-shape-registry.md`).
//!
//! A shape is a declared JSON Schema governing a data-artifact family within ONE home, keyed per
//! home so a shape never verdicts data its declarer cannot read. The registry revises by
//! assert/fold: amending a shape folds the prior row and inserts a new one; `shape_version` is the
//! chain depth.
//!
//! What is actually unknown here, and therefore what these pin:
//!
//! 1. **That a declared shape is found in force for a resource homed there.** The
//!    `_data_artifact_shape_in_force` resolver returns it via the incumbent `_data_artifact_anchor`
//!    and `_data_artifact_kind_owner` resolvers — never re-derived.
//! 2. **That a shape in one home is NOT in force in another.** The ruling's bite probe: two
//!    contexts, same `(kind_owner, kind)`. Declaring in C₁ must leave a resource homed in C₂
//!    reporting no shape in force. Remove `home_anchor_*` from the lookup and this test must fail.
//! 3. **The polymorphic arm.** A cogmap-homed resource finds a cogmap-homed shape.
//! 4. **Assert/fold.** Declaring twice folds the prior row and bumps `shape_version` to the chain
//!    depth; the folded row survives.
//!
//! Harness + seeding helpers follow the per-file convention of this suite.

mod common;

use temper_substrate::events::EventContext;
use temper_substrate::ids::{CogmapId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EnforcementMode, KindOwner};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams, DeclareShapeParams};
use uuid::Uuid;

// ── fixtures ──────────────────────────────────────────────────────────────────────────────────

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
    title: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: title,
            body: "seed body",
            doc_type: "research",
            home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// A world with one context-homed resource owned by `system`.
async fn world(pool: &sqlx::PgPool, slug: &str) -> (EntityId, ContextId, ResourceId, ProfileId) {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    );
    let resource = make_resource(pool, owner, emitter, AnchorRef::context(home), "subject").await;
    (emitter, home, resource, owner)
}

/// A world with one cogmap-homed resource owned by `system`.
async fn world_cogmap(
    pool: &sqlx::PgPool,
    name: &str,
) -> (EntityId, CogmapId, ResourceId, ProfileId) {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(pool, name, "Telos").await;
    let cogmap_id = CogmapId::from(cogmap);
    let resource = make_resource(
        pool,
        owner,
        emitter,
        AnchorRef::cogmap(cogmap_id),
        "cogmap-subject",
    )
    .await;
    (emitter, cogmap_id, resource, owner)
}

/// Declare a shape for `(home, kind_owner, kind)` via the Rust write path.
async fn declare(
    pool: &sqlx::PgPool,
    emitter: EntityId,
    home: AnchorRef,
    kind_owner: Option<KindOwner>,
    kind: &str,
    schema: &serde_json::Value,
    enforcement: EnforcementMode,
) -> Uuid {
    let id = writes::declare_shape(
        pool,
        DeclareShapeParams {
            home,
            kind,
            kind_owner,
            schema,
            enforcement,
            emitter,
        },
    )
    .await
    .unwrap();
    id.uuid()
}

/// `_data_artifact_shape_in_force` for a resource, as `(shape_id, shape_version, enforcement)`, or
/// `None` when no shape is in force.
async fn shape_in_force(
    pool: &sqlx::PgPool,
    resource: ResourceId,
    kind_owner_table: &str,
    kind_owner_id: Uuid,
    kind: &str,
) -> Option<(Uuid, i32, String)> {
    sqlx::query_as(
        "SELECT shape_id, shape_version, enforcement FROM _data_artifact_shape_in_force($1,$2,$3,$4)",
    )
    .bind(resource.uuid())
    .bind(kind_owner_table)
    .bind(kind_owner_id)
    .bind(kind)
    .fetch_optional(pool)
    .await
    .unwrap()
}

/// All rows in `kb_data_artifact_shapes` for `(home_anchor_table, home_anchor_id, kind_owner_table,
/// kind_owner_id, artifact_kind)`, as `(shape_id, shape_version, is_folded)`, in creation order.
async fn shape_rows(
    pool: &sqlx::PgPool,
    home_anchor_table: &str,
    home_anchor_id: Uuid,
    kind_owner_table: &str,
    kind_owner_id: Uuid,
    kind: &str,
) -> Vec<(Uuid, i32, bool)> {
    sqlx::query_as(
        "SELECT id, shape_version, is_folded FROM kb_data_artifact_shapes
          WHERE home_anchor_table=$1 AND home_anchor_id=$2
            AND kind_owner_table=$3 AND kind_owner_id=$4
            AND artifact_kind=$5
          ORDER BY created, id",
    )
    .bind(home_anchor_table)
    .bind(home_anchor_id)
    .bind(kind_owner_table)
    .bind(kind_owner_id)
    .bind(kind)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// A trivial JSON Schema that requires a `value` field of type string.
fn string_value_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "value": { "type": "string" }
        },
        "required": ["value"]
    })
}

// ── witnesses ─────────────────────────────────────────────────────────────────────────────────

/// `parse_shape_state` round-trips all four SQL literals to their typed variants, and an unknown
/// literal (including `""`) still returns `Err` — the `bail!` default is load-bearing: a `""` or
/// `NULL` is a decode error, not a silent "looks fine" (see `ShapeState`'s doc comment).
#[test]
fn parse_shape_state_round_trips_all_four_literals() {
    use temper_substrate::payloads::ShapeState;
    use temper_substrate::readback::parse_shape_state;

    assert_eq!(
        parse_shape_state("never_declared").unwrap(),
        ShapeState::NeverDeclared,
    );
    assert_eq!(
        parse_shape_state("declared_satisfied").unwrap(),
        ShapeState::DeclaredSatisfied,
    );
    assert_eq!(
        parse_shape_state("declared_not_satisfied").unwrap(),
        ShapeState::DeclaredNotSatisfied,
    );
    assert_eq!(
        parse_shape_state("declared_not_yet_checked").unwrap(),
        ShapeState::DeclaredNotYetChecked,
    );
}

/// The `bail!` default is load-bearing and must survive: an unknown literal (including `""` and
/// `NULL`-as-empty-string) is a decode error, not a silent "looks fine."
#[test]
fn parse_shape_state_rejects_unknown_literals() {
    use temper_substrate::readback::parse_shape_state;

    assert!(parse_shape_state("").is_err(), "empty string must error");
    assert!(
        parse_shape_state("bogus").is_err(),
        "unknown literal must error"
    );
    assert!(
        parse_shape_state("NeverDeclared").is_err(),
        "case mismatch must error — the SQL literals are snake_case"
    );
}

/// Declare for `(home, owner, kind)`; `_data_artifact_shape_in_force` returns it for a resource
/// homed there.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_shape_is_declared_and_found_in_force(pool: sqlx::PgPool) {
    let (emitter, home, resource, owner) = world(&pool, "home-a").await;
    let kind = "measurement";
    let schema = string_value_schema();

    let shape_id = declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema,
        EnforcementMode::Advisory,
    )
    .await;

    // The resource is homed in `home`, so the shape is in force for it.
    let in_force = shape_in_force(&pool, resource, "kb_profiles", owner.uuid(), kind)
        .await
        .expect("shape should be in force for a resource homed in the declaring context");
    assert_eq!(in_force.0, shape_id);
    assert_eq!(in_force.1, 1, "first declaration is version 1");
    assert_eq!(in_force.2, "advisory");
}

/// **The ruling's bite probe.** Two contexts, same `(kind_owner, kind)`. Declaring in C₁ must leave
/// a resource homed in C₂ reporting no shape in force. Remove `home_anchor_*` from the lookup and
/// this test must fail.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_shape_in_one_home_is_not_in_force_in_another(pool: sqlx::PgPool) {
    let (emitter, home_a, _resource_a, owner) = world(&pool, "home-a").await;
    let home_b = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "home-b", "home-b")
            .await
            .unwrap(),
    );
    let resource_b = make_resource(
        &pool,
        owner,
        emitter,
        AnchorRef::context(home_b),
        "subject-b",
    )
    .await;
    let kind = "measurement";
    let schema = string_value_schema();

    // Declare ONLY in home_a, naming the same owner+kind.
    declare(
        &pool,
        emitter,
        AnchorRef::context(home_a),
        Some(KindOwner::Profile(owner.uuid())),
        kind,
        &schema,
        EnforcementMode::Advisory,
    )
    .await;

    // A resource homed in home_b must report NO shape in force — the shape in home_a does not
    // reach across homes. This is ruling 2's bite: widen the lookup to `(kind_owner, kind)` and
    // this assertion fails.
    let in_force = shape_in_force(&pool, resource_b, "kb_profiles", owner.uuid(), kind).await;
    assert!(
        in_force.is_none(),
        "a shape declared in home_a must not be in force for a resource homed in home_b"
    );
}

/// The polymorphic arm — a cogmap-homed resource finds a cogmap-homed shape.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cogmap_homed_resource_resolves_its_shape(pool: sqlx::PgPool) {
    let (emitter, cogmap, resource, owner) = world_cogmap(&pool, "cogmap-home").await;
    let kind = "measurement";
    let schema = string_value_schema();

    let shape_id = declare(
        &pool,
        emitter,
        AnchorRef::cogmap(cogmap),
        None,
        kind,
        &schema,
        EnforcementMode::Enforcing,
    )
    .await;

    let in_force = shape_in_force(&pool, resource, "kb_profiles", owner.uuid(), kind)
        .await
        .expect("a cogmap-homed resource should find a cogmap-homed shape");
    assert_eq!(in_force.0, shape_id);
    assert_eq!(in_force.1, 1);
    assert_eq!(in_force.2, "enforcing");
}

/// Assert/fold; the folded row survives, `shape_version` is chain depth.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn declaring_twice_folds_the_prior_and_bumps_the_version(pool: sqlx::PgPool) {
    let (emitter, home, _resource, owner) = world(&pool, "home-a").await;
    let kind = "measurement";
    let schema_v1 = string_value_schema();
    let schema_v2 = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "value": { "type": "string" },
            "unit": { "type": "string" }
        },
        "required": ["value", "unit"]
    });

    let first = declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema_v1,
        EnforcementMode::Advisory,
    )
    .await;
    let second = declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema_v2,
        EnforcementMode::Enforcing,
    )
    .await;

    // Two rows for the same family in the same home; the first is folded, the second is live.
    let rows = shape_rows(
        &pool,
        "kb_contexts",
        home.uuid(),
        "kb_profiles",
        owner.uuid(),
        kind,
    )
    .await;
    assert_eq!(rows.len(), 2, "two declarations, two rows (assert/fold)");
    assert_eq!(rows[0].0, first, "the first row is the first declaration");
    assert!(
        rows[0].2,
        "the first row is folded by the second declaration"
    );
    assert_eq!(rows[0].1, 1, "the folded row keeps its version");
    assert_eq!(
        rows[1].0, second,
        "the second row is the second declaration"
    );
    assert!(!rows[1].2, "the second row is the live (non-folded) shape");
    assert_eq!(
        rows[1].1, 2,
        "the second row's version is the chain depth (2)"
    );

    // The shape in force is the second (live) declaration.
    let in_force = shape_in_force(&pool, _resource, "kb_profiles", owner.uuid(), kind)
        .await
        .expect("a live shape should be in force");
    assert_eq!(in_force.0, second);
    assert_eq!(in_force.1, 2);
    assert_eq!(in_force.2, "enforcing");
}
