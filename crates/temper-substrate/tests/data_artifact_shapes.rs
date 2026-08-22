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

use temper_core::types::home::HomeAnchor;
use temper_substrate::events::{fire, EventContext, Fired, SeedAction};
use temper_substrate::ids::{
    CogmapId, ContextId, DataArtifactId, EntityId, ProfileId, ResourceId, ShapeId,
};
use temper_substrate::payloads::{
    AnchorRef, ArtifactIntent, EnforcementMode, KindOwner, ShapeState,
};
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

/// Commit one data artifact via `fire()` and return the artifact id + conformance verdict.
async fn commit(
    pool: &sqlx::PgPool,
    resource: ResourceId,
    kind: &str,
    content: &serde_json::Value,
    emitter: EntityId,
) -> (DataArtifactId, ShapeState) {
    let mut tx = pool.begin().await.unwrap();
    let fired = fire(
        &mut tx,
        SeedAction::DataArtifactCommit {
            resource,
            kind,
            kind_owner: None,
            intent: ArtifactIntent::Current,
            precedence: 0.0,
            content,
            supersedes: &[],
            emitter,
        },
    )
    .await
    .unwrap();
    let (id, shape_state) = match fired {
        Fired::DataArtifact {
            artifact,
            shape_state,
        } => (artifact, shape_state),
        other => panic!("expected Fired::DataArtifact, got {other:?}"),
    };
    tx.commit().await.unwrap();
    (id, shape_state)
}

/// Attempt to commit; expect a refusal (error). Returns the error message.
async fn commit_err(
    pool: &sqlx::PgPool,
    resource: ResourceId,
    kind: &str,
    content: &serde_json::Value,
    emitter: EntityId,
) -> String {
    let mut tx = pool.begin().await.unwrap();
    let result = fire(
        &mut tx,
        SeedAction::DataArtifactCommit {
            resource,
            kind,
            kind_owner: None,
            intent: ArtifactIntent::Current,
            precedence: 0.0,
            content,
            supersedes: &[],
            emitter,
        },
    )
    .await;
    let err = result.expect_err("commit should have been refused");
    tx.rollback().await.ok();
    err.to_string()
}

/// Count artifact rows for a resource (to verify no row was written on refusal).
async fn artifact_count(pool: &sqlx::PgPool, resource: ResourceId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_data_artifacts WHERE resource_id=$1")
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
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

// ── Beat B: commit-time conformance verdict (Task 3) ─────────────────────────────────────────

/// A conforming commit under an advisory shape records `DeclaredSatisfied` synchronously.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_conforming_commit_records_declared_satisfied(pool: sqlx::PgPool) {
    let (emitter, home, resource, _owner) = world(&pool, "conform-ok").await;
    let kind = "measurement";
    let schema = string_value_schema();

    declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema,
        EnforcementMode::Advisory,
    )
    .await;

    let conforming = serde_json::json!({"value": "42"});
    let (_id, shape_state) = commit(&pool, resource, kind, &conforming, emitter).await;

    assert_eq!(
        shape_state,
        ShapeState::DeclaredSatisfied,
        "a conforming commit must record DeclaredSatisfied synchronously"
    );
}

/// **The posture's bite probe.** An advisory shape records non-conformance WITHOUT refusing —
/// the commit succeeds, the artifact is retrievable, and the verdict is `DeclaredNotSatisfied`.
/// If this ever refuses, ruling 4 has been lost.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_advisory_shape_records_non_conformance_without_refusing(pool: sqlx::PgPool) {
    let (emitter, home, resource, _owner) = world(&pool, "advisory-bite").await;
    let kind = "measurement";
    let schema = string_value_schema();

    declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema,
        EnforcementMode::Advisory,
    )
    .await;

    // Non-conforming: `value` is a number, not a string.
    let non_conforming = serde_json::json!({"value": 42});
    let (id, shape_state) = commit(&pool, resource, kind, &non_conforming, emitter).await;

    assert_eq!(
        shape_state,
        ShapeState::DeclaredNotSatisfied,
        "advisory non-conformance must record DeclaredNotSatisfied, not refuse"
    );

    // The artifact IS retrievable — the commit succeeded.
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM kb_data_artifacts WHERE id=$1 AND NOT is_folded")
            .bind(id.uuid())
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert!(
        row.is_some(),
        "the artifact row must exist under advisory non-conformance — the commit succeeded"
    );
}

/// An enforcing shape refuses a non-conforming commit, carries the validation failure, and writes
/// NO artifact row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_enforcing_shape_refuses_and_says_what_failed(pool: sqlx::PgPool) {
    let (emitter, home, resource, _owner) = world(&pool, "enforcing-refuse").await;
    let kind = "measurement";
    let schema = string_value_schema();

    declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema,
        EnforcementMode::Enforcing,
    )
    .await;

    let non_conforming = serde_json::json!({"value": 42});
    let err = commit_err(&pool, resource, kind, &non_conforming, emitter).await;

    assert!(
        err.contains("does not conform"),
        "the refusal must say the content does not conform, got: {err}"
    );

    // No artifact row was written.
    let count = artifact_count(&pool, resource).await;
    assert_eq!(
        count, 0,
        "no artifact row must be written when an enforcing shape refuses"
    );
}

/// A commit with no shape in force stays `NeverDeclared` — persistence never requires a prior
/// declaration.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_commit_with_no_shape_in_force_stays_never_declared(pool: sqlx::PgPool) {
    let (emitter, _home, resource, _owner) = world(&pool, "no-shape").await;
    let kind = "measurement";

    // No shape declared — just commit.
    let content = serde_json::json!({"value": "42"});
    let (_id, shape_state) = commit(&pool, resource, kind, &content, emitter).await;

    assert_eq!(
        shape_state,
        ShapeState::NeverDeclared,
        "a commit with no shape in force must stay NeverDeclared"
    );
}

// ── Beat C: verdict read-model and the staleness triple (Task 4) ──────────────

/// Read `shape_state` from the SQL read function `artifacts_for_resource` for the first live
/// artifact of `resource`, or `None` when the resource has no live artifacts.
async fn read_shape_state(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    resource: ResourceId,
) -> Option<ShapeState> {
    let retrieved = temper_substrate::readback::artifacts_for_resource(
        pool, principal, resource, None, None, false,
    )
    .await
    .unwrap();
    retrieved.first().map(|a| a.shape_state)
}

/// Fold the shape (re-declare) so `shape_version` bumps; the old verdict row still exists but the
/// artifact reports `DeclaredNotYetChecked` because the staleness triple no longer matches.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_stale_verdict_reads_as_not_yet_checked(pool: sqlx::PgPool) {
    let (emitter, home, resource, owner) = world(&pool, "stale-verdict").await;
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

    // Declare v1, commit a conforming artifact — verdict is DeclaredSatisfied.
    declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema_v1,
        EnforcementMode::Advisory,
    )
    .await;

    let conforming = serde_json::json!({"value": "42"});
    let (artifact_id, shape_state) = commit(&pool, resource, kind, &conforming, emitter).await;
    assert_eq!(
        shape_state,
        ShapeState::DeclaredSatisfied,
        "commit under v1 shape should be satisfied"
    );

    // The read path must also report DeclaredSatisfied — the verdict row matches the triple.
    let read_state = read_shape_state(&pool, owner, resource)
        .await
        .expect("artifact should be visible");
    assert_eq!(
        read_state,
        ShapeState::DeclaredSatisfied,
        "read path should report DeclaredSatisfied when the verdict triple matches"
    );

    // Fold the shape (re-declare → version bumps to 2). The old verdict row still exists
    // (artifact_id is PK, not touched) but its shape_version=1 no longer matches the live v2.
    declare(
        &pool,
        emitter,
        AnchorRef::context(home),
        None,
        kind,
        &schema_v2,
        EnforcementMode::Advisory,
    )
    .await;

    // The verdict row still exists — it was not deleted.
    let verdict_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_verdicts WHERE artifact_id=$1")
            .bind(artifact_id.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        verdict_count, 1,
        "the old verdict row must still exist — staleness is by triple mismatch, not deletion"
    );

    // But the read path reports DeclaredNotYetChecked — the triple no longer matches.
    let read_state = read_shape_state(&pool, owner, resource)
        .await
        .expect("artifact should still be visible");
    assert_eq!(
        read_state,
        ShapeState::DeclaredNotYetChecked,
        "a stale verdict (shape_version mismatch) must read as DeclaredNotYetChecked"
    );

    // Now upsert a verdict with the CORRECT (shape_id, shape_version) for the live v2 shape
    // but a WRONG content_hash. The content_hash leg of the triple must catch this: the
    // read path should still report DeclaredNotYetChecked. Drop content_hash from the triple
    // and this assertion fails — the wrong verdict would be trusted as DeclaredSatisfied.
    let live_shape = shape_in_force(&pool, resource, "kb_profiles", owner.uuid(), kind)
        .await
        .expect("a live shape should be in force after the fold");
    sqlx::query!(
        "SELECT data_artifact_verdict_upsert($1,$2,$3,$4,$5,$6)",
        artifact_id.uuid(),
        live_shape.0,
        live_shape.1,
        "deadbeef0000000000000000000000000000000000000000000000000000dead",
        true,
        None::<serde_json::Value>,
    )
    .execute(&pool)
    .await
    .unwrap();

    let read_state = read_shape_state(&pool, owner, resource)
        .await
        .expect("artifact should still be visible");
    assert_eq!(
        read_state,
        ShapeState::DeclaredNotYetChecked,
        "a verdict with a wrong content_hash must read as DeclaredNotYetChecked — \
         the content_hash leg of the staleness triple is load-bearing"
    );
}

/// `resource_rehome` to a context with no shape for this family; the artifact reports
/// `DeclaredNotYetChecked` (a shape was declared in the old home, but none is in force in the
/// new home). The verdict row is not deleted — staleness is by triple mismatch, not deletion.
///
/// Wait — re-reading the plan: "to a context with a different shape". If there's NO shape in the
/// new home, the artifact should report `NeverDeclared` (no shape in force at all). The test should
/// rehome to a context that HAS a different shape, so the `shape_id` changes and the verdict
/// becomes stale. Let me re-read the plan assertion:
///
/// > `resource_rehome` to a context with a different shape; artifacts report
/// > `DeclaredNotYetChecked` without any verdict row being deleted
///
/// So we need a different shape in the new home — not no shape. The shape_id changes, so the
/// triple mismatches and the verdict reads as stale.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn rehoming_a_resource_invalidates_its_verdicts(pool: sqlx::PgPool) {
    let (emitter, home_a, resource, owner) = world(&pool, "rehome-a").await;
    let kind = "measurement";
    let schema = string_value_schema();

    // Declare a shape in home_a.
    let shape_a = declare(
        &pool,
        emitter,
        AnchorRef::context(home_a),
        None,
        kind,
        &schema,
        EnforcementMode::Advisory,
    )
    .await;

    // Commit a conforming artifact — verdict is DeclaredSatisfied.
    let conforming = serde_json::json!({"value": "42"});
    let (artifact_id, shape_state) = commit(&pool, resource, kind, &conforming, emitter).await;
    assert_eq!(shape_state, ShapeState::DeclaredSatisfied);

    // Create a second context (home_b) and declare a different shape there. A resource must
    // exist in home_b for the wrapper's kind_owner defaulting to resolve.
    let home_b = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "rehome-b", "rehome-b")
            .await
            .unwrap(),
    );
    let _dummy = make_resource(
        &pool,
        owner,
        emitter,
        AnchorRef::context(home_b),
        "rehome-dummy",
    )
    .await;
    let shape_b = declare(
        &pool,
        emitter,
        AnchorRef::context(home_b),
        None,
        kind,
        &schema,
        EnforcementMode::Advisory,
    )
    .await;
    assert_ne!(shape_a, shape_b, "the two shapes must be different");

    // Rehome the resource to home_b.
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceRehome {
            resource,
            home: AnchorRef::context(home_b),
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // The verdict row still exists — it was not deleted.
    let verdict_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_verdicts WHERE artifact_id=$1")
            .bind(artifact_id.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        verdict_count, 1,
        "the verdict row must still exist after rehome — staleness is by triple mismatch"
    );

    // The read path reports DeclaredNotYetChecked — shape_id changed (now shape_b governs),
    // so the stored verdict (against shape_a) is stale.
    let read_state = read_shape_state(&pool, owner, resource)
        .await
        .expect("artifact should still be visible after rehome");
    assert_eq!(
        read_state,
        ShapeState::DeclaredNotYetChecked,
        "after rehome to a different shape, the verdict is stale and must read as \
         DeclaredNotYetChecked"
    );
}

/// An artifact whose family has no shape declared reports `NeverDeclared` — the existing behaviour,
/// guarded as a regression.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_artifact_with_no_shape_reports_never_declared(pool: sqlx::PgPool) {
    let (emitter, _home, resource, owner) = world(&pool, "no-shape-read").await;
    let kind = "measurement";

    // No shape declared — just commit.
    let content = serde_json::json!({"value": "42"});
    let (_id, shape_state) = commit(&pool, resource, kind, &content, emitter).await;

    assert_eq!(
        shape_state,
        ShapeState::NeverDeclared,
        "commit-time verdict with no shape must be NeverDeclared"
    );

    // The read path must also report NeverDeclared.
    let read_state = read_shape_state(&pool, owner, resource)
        .await
        .expect("artifact should be visible");
    assert_eq!(
        read_state,
        ShapeState::NeverDeclared,
        "read path with no shape in force must report NeverDeclared"
    );
}

// ── Beat C: registry enumeration reads, visibility-gated (Task 5) ─────────────────────────────

/// A principal who cannot read the home context sees zero shapes from `shapes_for_home`, and
/// `shape_by_id` returns `None` for a shape id in that context. The owner sees both. This is the
/// visibility gate's bite probe: remove the `anchor_readable_by_profile` predicate from the SQL
/// and the stranger sees the shape — this assertion fails.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn shape_reads_gate_on_home_visibility(pool: sqlx::PgPool) {
    let (emitter, home, _resource, owner) = world(&pool, "gate-home").await;
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

    // A stranger — a profile with no grants, no team membership, and no ownership of the home
    // context — cannot read it. `anchor_readable_by_profile` returns false for this profile.
    let stranger = ProfileId::from(common::insert_profile(&pool, "shape-gate-stranger").await);

    // The owner sees the shape via `shapes_for_home`.
    let owner_shapes =
        temper_substrate::readback::shapes_for_home(&pool, owner, HomeAnchor::Context(home))
            .await
            .unwrap();
    assert_eq!(
        owner_shapes.len(),
        1,
        "the owner should see one live shape in their own context"
    );
    assert_eq!(owner_shapes[0].shape_id.uuid(), shape_id);

    // The stranger sees zero shapes — the visibility gate filters them out.
    let stranger_shapes =
        temper_substrate::readback::shapes_for_home(&pool, stranger, HomeAnchor::Context(home))
            .await
            .unwrap();
    assert!(
        stranger_shapes.is_empty(),
        "a stranger who cannot read the home context must see zero shapes from shapes_for_home"
    );

    // The owner sees the shape via `shape_by_id`.
    let owner_shape =
        temper_substrate::readback::shape_by_id(&pool, owner, ShapeId::from(shape_id))
            .await
            .unwrap();
    assert!(
        owner_shape.is_some(),
        "the owner should see the shape by id in their own context"
    );

    // The stranger sees nothing — `shape_by_id` returns `None` (fail closed).
    let stranger_shape =
        temper_substrate::readback::shape_by_id(&pool, stranger, ShapeId::from(shape_id))
            .await
            .unwrap();
    assert!(
        stranger_shape.is_none(),
        "a stranger who cannot read the home context must get None from shape_by_id — fail closed"
    );
}
