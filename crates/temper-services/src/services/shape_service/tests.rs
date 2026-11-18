use super::*;
use sqlx::PgPool;
use temper_substrate::events::{fire, EventContext, Fired, SeedAction};
use temper_substrate::ids::{DataArtifactId, ResourceId};
use temper_substrate::payloads::{ArtifactIntent, ShapeState};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};

// ── seeding helpers ──────────────────────────────────────────────────────

async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
    crate::test_support::profile::insert(pool, handle).await
}

async fn insert_entity(pool: &PgPool, profile: Uuid, name: &str) -> Uuid {
    crate::test_support::entity::insert(pool, profile, name).await
}

async fn insert_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    crate::test_support::context::insert_personal(pool, owner, slug, slug).await
}

async fn make_resource(
    pool: &PgPool,
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

/// Seed a world with one context-homed resource owned by `owner`.
struct World {
    owner: Uuid,
    emitter: Uuid,
    context: Uuid,
    resource: Uuid,
}

async fn seed_world(pool: &PgPool, slug: &str) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let owner = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let emitter = insert_entity(pool, owner, "test-emitter").await;
    let context = insert_context(pool, owner, slug).await;
    let resource = make_resource(
        pool,
        ProfileId::from(owner),
        EntityId::from(emitter),
        AnchorRef::context(ContextId::from(context)),
        "subject",
    )
    .await;
    World {
        owner,
        emitter,
        context,
        resource: resource.uuid(),
    }
}

/// Commit one data artifact via `fire()` and return the artifact id + conformance verdict.
async fn commit_artifact(
    pool: &PgPool,
    resource: ResourceId,
    kind: &str,
    content: &serde_json::Value,
    emitter: Uuid,
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
            emitter: EntityId::from(emitter),
        },
    )
    .await
    .unwrap();
    let (id, shape_state) = match fired {
        Fired::DataArtifact {
            artifact,
            shape_state,
            ..
        } => (artifact, shape_state),
        other => panic!("expected Fired::DataArtifact, got {other:?}"),
    };
    tx.commit().await.unwrap();
    (id, shape_state)
}

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

/// Count shape-reconcile jobs on the anchor queue for a given anchor.
async fn reconcile_job_count(pool: &PgPool, anchor_table: &str, anchor_id: Uuid) -> i64 {
    let (cogmap, context) = match anchor_table {
        "kb_cogmaps" => (Some(anchor_id), None::<Uuid>),
        _ => (None, Some(anchor_id)),
    };
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_workflow_jobs \
         WHERE cogmap_id IS NOT DISTINCT FROM $1 \
           AND context_id IS NOT DISTINCT FROM $2 \
           AND persona = 'shape' \
           AND dispatch_type = 'shape-reconcile'",
    )
    .bind(cogmap)
    .bind(context)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Read the shape_state from the SQL read function for the first live artifact of `resource`.
async fn read_shape_state(
    pool: &PgPool,
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

// ── witnesses ─────────────────────────────────────────────────────────────

/// Declaring a shape enqueues exactly one reconcile job, anchored to the shape's home.
#[sqlx::test(migrations = "../../migrations")]
async fn declaring_a_shape_enqueues_one_reconcile_job(pool: PgPool) {
    let w = seed_world(&pool, "enqueue-test").await;
    let kind = "measurement";
    let schema = string_value_schema();

    let shape_id = declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    assert!(!shape_id.uuid().is_nil(), "shape was declared");
    assert_eq!(
        reconcile_job_count(&pool, "kb_contexts", w.context).await,
        1,
        "exactly one shape-reconcile job anchored to the shape's home"
    );
}

/// Declaring twice collapses to one in-flight job — the single-flight index does its job.
/// The second enqueue returns `None`, not an error.
#[sqlx::test(migrations = "../../migrations")]
async fn declaring_twice_collapses_to_one_in_flight_job(pool: PgPool) {
    let w = seed_world(&pool, "dedup-test").await;
    let kind = "measurement";
    let schema = string_value_schema();
    let home = AnchorRef::context(ContextId::from(w.context));

    // First declare — enqueues a job.
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home,
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    // Second declare — the single-flight index collapses: still one job, not two.
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home,
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        reconcile_job_count(&pool, "kb_contexts", w.context).await,
        1,
        "two declarations collapse to one in-flight job — the single-flight index works"
    );
}

/// Reconciliation verdicts the pre-existing backlog: artifacts committed BEFORE the declaration
/// move from `DeclaredNotYetChecked` to a real verdict.
#[sqlx::test(migrations = "../../migrations")]
async fn reconciliation_verdicts_the_pre_existing_backlog(pool: PgPool) {
    let w = seed_world(&pool, "reconcile-test").await;
    let kind = "measurement";
    let resource = ResourceId::from(w.resource);

    // Commit an artifact BEFORE declaring any shape — it starts as NeverDeclared.
    let conforming = serde_json::json!({"value": "42"});
    let (_artifact_id, shape_state) =
        commit_artifact(&pool, resource, kind, &conforming, w.emitter).await;
    assert_eq!(
        shape_state,
        ShapeState::NeverDeclared,
        "commit before any shape declared must be NeverDeclared"
    );

    // Declare a shape — this enqueues a reconcile job.
    let schema = string_value_schema();
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    // The pre-existing artifact now reads as DeclaredNotYetChecked — a shape is in force but
    // no verdict has been recorded yet.
    let read_state = read_shape_state(&pool, ProfileId::from(w.owner), resource)
        .await
        .expect("artifact should be visible");
    assert_eq!(
        read_state,
        ShapeState::DeclaredNotYetChecked,
        "after declare, the pre-existing artifact must read as DeclaredNotYetChecked"
    );

    // Run the reconciler — it should verdict the backlog.
    let count = dispatch_tick(&pool, None).await.unwrap().verdicts_written;

    assert_eq!(
        count, 1,
        "exactly one verdict must be written — the pre-existing artifact"
    );

    // The artifact now reads as DeclaredSatisfied — the conforming content was validated.
    let read_state = read_shape_state(&pool, ProfileId::from(w.owner), resource)
        .await
        .expect("artifact should still be visible");
    assert_eq!(
        read_state,
        ShapeState::DeclaredSatisfied,
        "after reconciliation, the conforming artifact must read as DeclaredSatisfied"
    );
}

/// Reconciliation records ONE violation per artifact, not every one: content breaking three
/// properties gets a stored detail with exactly one entry.
#[sqlx::test(migrations = "../../migrations")]
async fn reconcile_records_one_violation(pool: PgPool) {
    let w = seed_world(&pool, "reconcile-one-violation").await;
    let kind = "measurement";
    let resource = ResourceId::from(w.resource);

    let content = serde_json::json!({"alpha": "a", "bravo": "b", "charlie": "c"});
    let (artifact_id, _) = commit_artifact(&pool, resource, kind, &content, w.emitter).await;

    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "alpha": { "type": "string", "minLength": 5 },
            "bravo": { "type": "string", "minLength": 5 },
            "charlie": { "type": "string", "minLength": 5 }
        },
        "required": ["alpha", "bravo", "charlie"]
    });
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    let summary = dispatch_tick(&pool, None).await.unwrap();
    assert_eq!(
        summary.verdicts_written, 1,
        "the one pre-existing artifact is verdicted"
    );

    let detail: serde_json::Value =
        sqlx::query_scalar("SELECT detail FROM kb_data_artifact_verdicts WHERE artifact_id = $1")
            .bind(artifact_id.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        detail.as_array().map(Vec::len),
        Some(1),
        "the stored detail has exactly one entry, got: {detail}"
    );
}

/// A reconcile tick that claims zero jobs must not be mistakable for a successful sweep —
/// assert on verdicts written, not on the tick returning Ok. An empty anchor (no jobs queued)
/// must write zero verdicts.
#[sqlx::test(migrations = "../../migrations")]
async fn a_reconcile_tick_with_no_work_touches_nothing(pool: PgPool) {
    let w = seed_world(&pool, "no-work-test").await;
    let kind = "measurement";
    let resource = ResourceId::from(w.resource);

    // Commit an artifact but do NOT declare a shape — no reconcile job is queued.
    let conforming = serde_json::json!({"value": "42"});
    commit_artifact(&pool, resource, kind, &conforming, w.emitter).await;

    // No jobs on the queue.
    assert_eq!(
        reconcile_job_count(&pool, "kb_contexts", w.context).await,
        0,
        "no shape declared → no reconcile job queued"
    );

    // Run the reconciler — it claims zero jobs and writes zero verdicts.
    let count = dispatch_tick(&pool, None).await.unwrap().verdicts_written;

    assert_eq!(
        count, 0,
        "a tick with no work must write zero verdicts — not be mistakable for a successful sweep"
    );

    // No verdict rows exist.
    let verdict_count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_verdicts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        verdict_count, 0,
        "no verdict rows must exist when no reconciliation ran"
    );
}

/// A principal who cannot author the home context is refused — the authority gate is
/// load-bearing. A stranger (no team membership, no ownership) gets `Forbidden`.
#[sqlx::test(migrations = "../../migrations")]
async fn declaring_a_shape_requires_authority_over_the_home(pool: PgPool) {
    let w = seed_world(&pool, "auth-gate-test").await;
    let kind = "measurement";
    let schema = string_value_schema();

    // A stranger — no team membership, no ownership of the home context.
    let stranger = insert_profile(&pool, "shape-stranger").await;

    let result = declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(stranger),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await;

    assert!(
        result.is_err(),
        "a stranger who cannot author the home context must be refused"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, ApiError::Forbidden),
        "the refusal must be Forbidden, got {err:?}"
    );

    // No shape was declared and no job was enqueued.
    let shape_count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_shapes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(shape_count, 0, "no shape row must exist after a refusal");

    assert_eq!(
        reconcile_job_count(&pool, "kb_contexts", w.context).await,
        0,
        "no reconcile job must be enqueued after a refusal"
    );

    // The owner CAN declare — the gate admits them.
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("the owner must be able to declare a shape in their own context");
}

/// Run the erasure act on `resource` as `operator`, through the SQL act directly (the service
/// gate is the admin surface's concern, not reconcile's).
async fn erase(pool: &PgPool, resource: Uuid, operator: Uuid, emitter: Uuid) {
    sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(resource)
        .bind(operator)
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(pool)
        .await
        .expect("the erasure act completes");
}

/// The `shape_version` of the verdict stored for `artifact`, if any.
async fn verdict_version(pool: &PgPool, artifact: DataArtifactId) -> Option<i32> {
    sqlx::query_scalar("SELECT shape_version FROM kb_data_artifact_verdicts WHERE artifact_id = $1")
        .bind(artifact.uuid())
        .fetch_optional(pool)
        .await
        .unwrap()
}

/// The statuses of every shape-reconcile job on a context anchor.
async fn reconcile_job_statuses(pool: &PgPool, context: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT status::text FROM kb_workflow_jobs \
          WHERE context_id = $1 AND persona = 'shape' AND dispatch_type = 'shape-reconcile' \
          ORDER BY enqueued_at, id",
    )
    .bind(context)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// An erased resource's artifact never blocks reconcile for its home. The guarded verdict
/// writer refuses it (D13), so the backlog must not reach it: the sibling is verdicted, the
/// anchor's job completes, and the erased artifact gains no verdict.
///
/// The erased resource's artifact is committed first, so it leads the backlog; a filter that
/// merely let the sibling go first would still leave the anchor in flight, which the job
/// status and the returned count catch whatever the order.
#[sqlx::test(migrations = "../../migrations")]
async fn an_erased_resources_artifact_never_blocks_reconcile_for_its_home(pool: PgPool) {
    let w = seed_world(&pool, "erased-reconcile").await;
    let kind = "measurement";
    let home = AnchorRef::context(ContextId::from(w.context));
    let erased = ResourceId::from(w.resource);
    let sibling = make_resource(
        &pool,
        ProfileId::from(w.owner),
        EntityId::from(w.emitter),
        home,
        "sibling",
    )
    .await;

    // 1. A shape in force in the home.
    let declare = |schema: serde_json::Value| {
        let pool = pool.clone();
        async move {
            declare_shape(
                &pool,
                DeclareShapeServiceParams {
                    home,
                    kind,
                    kind_owner: None,
                    schema: &schema,
                    enforcement: EnforcementMode::Advisory,
                    principal: ProfileId::from(w.owner),
                    emitter: EntityId::from(w.emitter),
                },
            )
            .await
            .unwrap();
        }
    };
    declare(string_value_schema()).await;
    assert_eq!(
        dispatch_tick(&pool, None).await.unwrap().verdicts_written,
        0,
        "the first declare's job finds an empty backlog"
    );

    // 2. One artifact of the family on each resource, the erased-to-be one first.
    let content = serde_json::json!({"value": "42"});
    let (erased_artifact, _) = commit_artifact(&pool, erased, kind, &content, w.emitter).await;
    let (sibling_artifact, _) = commit_artifact(&pool, sibling, kind, &content, w.emitter).await;

    // 3. Erase the first resource.
    erase(&pool, w.resource, w.owner, w.emitter).await;
    let erased_before = verdict_version(&pool, erased_artifact).await;

    // 4. Amend the shape: both triples go stale, so both artifacts are in the backlog.
    let mut amended = string_value_schema();
    amended["properties"]["unit"] = serde_json::json!({ "type": "string" });
    declare(amended).await;

    let written = dispatch_tick(&pool, None).await.unwrap().verdicts_written;

    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["done".to_owned(), "done".to_owned()],
        "the amended shape's reconcile job completes; it is not left in flight for the reaper"
    );
    assert_eq!(written, 1, "exactly one verdict is written: the sibling's");
    assert_eq!(
        verdict_version(&pool, sibling_artifact).await,
        Some(2),
        "the sibling is verdicted against the amended shape"
    );
    assert_eq!(
        verdict_version(&pool, erased_artifact).await,
        erased_before,
        "the erased resource's artifact gains no verdict"
    );
}

/// Declare the world's `measurement` shape over a backlog of `n` artifacts, one per resource,
/// committed before the declare. Returns the artifacts in commit order.
async fn backlog_of(pool: &PgPool, w: &World, n: usize) -> Vec<DataArtifactId> {
    let home = AnchorRef::context(ContextId::from(w.context));
    let content = serde_json::json!({"value": "42"});
    let mut artifacts = Vec::with_capacity(n);
    for i in 0..n {
        let resource = make_resource(
            pool,
            ProfileId::from(w.owner),
            EntityId::from(w.emitter),
            home,
            &format!("backlog-{}-{i}", w.context),
        )
        .await;
        let (artifact, _) =
            commit_artifact(pool, resource, "measurement", &content, w.emitter).await;
        artifacts.push(artifact);
    }
    declare_shape(
        pool,
        DeclareShapeServiceParams {
            home,
            kind: "measurement",
            kind_owner: None,
            schema: &string_value_schema(),
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();
    artifacts
}

const ROOMY: std::time::Duration = std::time::Duration::from_secs(600);

/// FAILS IF: a pass reads one batch and stops, so a backlog larger than a batch is left
/// partly unverdicted with its job marked done.
#[sqlx::test(migrations = "../../migrations")]
async fn a_backlog_larger_than_a_batch_is_verdicted_across_batches(pool: PgPool) {
    let w = seed_world(&pool, "batched-reconcile").await;
    backlog_of(&pool, &w, 5).await;

    let summary = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(
            std::time::Instant::now(),
            ROOMY,
            BatchBound {
                rows: 2,
                bytes: i64::MAX,
                max_one: i64::MAX,
            },
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        summary.verdicts_written, 5,
        "every artifact of the backlog is verdicted"
    );
    assert_eq!(summary.completed, 1, "the anchor's job completes");
    assert_eq!(summary.partial, 0);
    assert_eq!(
        summary.batches_read, 3,
        "a two-row bound reads five artifacts as batches of two, two and one"
    );
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["done".to_owned()]
    );
}

/// FAILS IF: the byte bound admits no row when the first artifact alone is over it, so a batch
/// of large artifacts never moves forward; or admits a row past the bound after the first, so
/// the bound does not bound the batch.
#[sqlx::test(migrations = "../../migrations")]
async fn a_batch_always_admits_its_first_artifact_whatever_its_size(pool: PgPool) {
    let w = seed_world(&pool, "byte-bounded-reconcile").await;
    backlog_of(&pool, &w, 3).await;

    let summary = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(
            std::time::Instant::now(),
            ROOMY,
            BatchBound {
                rows: 100,
                bytes: 1,
                max_one: i64::MAX,
            },
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        summary.verdicts_written, 3,
        "a one-byte budget still reaches every artifact"
    );
    assert_eq!(
        summary.batches_read, 3,
        "a one-byte budget reads exactly one artifact a batch"
    );
    assert_eq!(summary.completed, 1);
}

/// FAILS IF: the byte bound admits the artifact that crosses it, so a batch can hold up to one
/// artifact more than the bound says.
#[sqlx::test(migrations = "../../migrations")]
async fn the_byte_bound_is_a_ceiling_on_a_batch_not_a_threshold(pool: PgPool) {
    let w = seed_world(&pool, "byte-ceiling-reconcile").await;
    backlog_of(&pool, &w, 2).await;
    // Both artifacts hold the same content, so one byte past one artifact admits one, and a
    // threshold reading of the bound would admit both.
    let one: i64 = sqlx::query_scalar(
        "SELECT max(octet_length(content::text))::bigint FROM kb_data_artifact_content",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let summary = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(
            std::time::Instant::now(),
            ROOMY,
            BatchBound {
                rows: 100,
                bytes: one + 1,
                max_one: i64::MAX,
            },
        ),
    )
    .await
    .unwrap();

    assert_eq!(summary.verdicts_written, 2);
    assert_eq!(
        summary.batches_read, 2,
        "the second artifact would take the batch past its bound, so it starts the next one"
    );
}

/// FAILS IF: a job claimed after the deadline is spent is worked on anyway, or is dropped
/// rather than handed back for the next pass.
#[sqlx::test(migrations = "../../migrations")]
async fn a_spent_deadline_hands_every_claimed_job_back_untouched(pool: PgPool) {
    let w = seed_world(&pool, "deferred-reconcile").await;
    backlog_of(&pool, &w, 1).await;

    let deferred = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(
            std::time::Instant::now(),
            std::time::Duration::ZERO,
            SHAPE_RECONCILE_BATCH,
        ),
    )
    .await
    .unwrap();
    assert_eq!(deferred.deferred, 1, "the one claimed job is handed back");
    assert_eq!(
        deferred.verdicts_written, 0,
        "no work is attempted past the deadline"
    );
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["done".to_owned(), "pending".to_owned()],
        "the claimed job is done and its successor waits for the next pass"
    );

    let next = dispatch_tick(&pool, None).await.unwrap();
    assert_eq!(
        next.verdicts_written, 1,
        "the next pass reconciles the anchor"
    );
    assert_eq!(next.completed, 1);
}

/// FAILS IF: an anchor cut off by the deadline mid-backlog loses the verdicts it wrote, or its
/// job is marked done with the rest of the backlog unreached.
#[sqlx::test(migrations = "../../migrations")]
async fn an_anchor_cut_off_mid_backlog_keeps_its_verdicts_and_resumes(pool: PgPool) {
    let w = seed_world(&pool, "partial-reconcile").await;
    backlog_of(&pool, &w, 5).await;

    let claimed = workflow_job_service::claim_anchor(
        &pool,
        Persona::Shape.as_str(),
        DispatchType::ShapeReconcile.as_str(),
        1,
        DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
    )
    .await
    .unwrap();
    let job = claimed.first().expect("the declare queued a job");

    // A deadline spent as soon as the pass starts. The job was not past it when it came up
    // (`past_deadline` false), so the pass reaches its first artifact, and the deadline is
    // spent at the check after it.
    let pace = Pace::new(
        std::time::Instant::now(),
        std::time::Duration::ZERO,
        BatchBound {
            rows: 2,
            bytes: i64::MAX,
            max_one: i64::MAX,
        },
    );
    let ran = run_reconcile_job(&pool, job, None, false, pace).await;
    let ReconcileJobResult::Ran { pass, settled } = ran else {
        panic!("the job ran a pass: {ran:?}");
    };
    assert_eq!(
        pass.stop,
        Stop::Deadline,
        "the deadline came before the backlog was exhausted"
    );
    assert_eq!(settled, Settled::HandedBack);
    assert_eq!(pass.verdicts_written, 1, "one artifact was verdicted");
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["done".to_owned(), "pending".to_owned()],
        "a partial anchor is handed back to resume"
    );

    let rest = dispatch_tick(&pool, None).await.unwrap();
    assert_eq!(
        rest.verdicts_written, 4,
        "the next pass verdicts only what is left"
    );
    assert_eq!(rest.completed, 1);
}

/// FAILS IF: an artifact committed with `null` content, which has no content row, fails its
/// anchor's pass instead of being verdicted as the commit verdicted it.
#[sqlx::test(migrations = "../../migrations")]
async fn an_artifact_with_null_content_is_verdicted_not_failed(pool: PgPool) {
    let w = seed_world(&pool, "null-content-reconcile").await;
    let resource = ResourceId::from(w.resource);
    let (artifact, _) = commit_artifact(
        &pool,
        resource,
        "measurement",
        &serde_json::Value::Null,
        w.emitter,
    )
    .await;
    let has_content_row: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_data_artifact_content WHERE artifact_id = $1)",
    )
    .bind(artifact.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !has_content_row,
        "precondition: null content stores no content row"
    );
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind: "measurement",
            kind_owner: None,
            schema: &string_value_schema(),
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(summary.failed, 0, "the anchor's pass does not fail");
    assert_eq!(summary.verdicts_written, 1);
    assert_eq!(
        read_shape_state(&pool, ProfileId::from(w.owner), resource).await,
        Some(ShapeState::DeclaredNotSatisfied),
        "null is validated against the shape as the commit validates it: an object is required"
    );
}

/// FAILS IF: a change that coalesced into a running job is lost when the job completes, so its
/// artifacts never gain a verdict for the shape now in force.
#[sqlx::test(migrations = "../../migrations")]
async fn a_change_that_coalesced_into_a_running_job_is_queued_again_when_it_completes(
    pool: PgPool,
) {
    let w = seed_world(&pool, "coalesced-reconcile").await;
    backlog_of(&pool, &w, 2).await;
    let claimed = workflow_job_service::claim_anchor(
        &pool,
        Persona::Shape.as_str(),
        DispatchType::ShapeReconcile.as_str(),
        1,
        DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
    )
    .await
    .unwrap();
    let job = claimed.first().expect("the declare queued a job");

    // The pass verdicts the backlog against version 1.
    let pace = Pace::new(std::time::Instant::now(), ROOMY, SHAPE_RECONCILE_BATCH);
    let pass = reconcile_one_anchor(&pool, job.anchor, pace).await.unwrap();
    assert_eq!(pass.verdicts_written, 2);

    // An amendment lands while the job is still in flight: its enqueue coalesces into it.
    let mut amended = string_value_schema();
    amended["properties"]["unit"] = serde_json::json!({ "type": "string" });
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind: "measurement",
            kind_owner: None,
            schema: &amended,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["in_progress".to_owned()],
        "precondition: the amendment's enqueue coalesced into the running job"
    );

    let settled = settle(&pool, job, &pass).await;

    assert_eq!(settled, Settled::Completed { requeued: true });
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["done".to_owned(), "pending".to_owned()],
        "the job is done, and a successor waits for the amended shape's backlog"
    );
    let next = dispatch_tick(&pool, None).await.unwrap();
    assert_eq!(
        next.verdicts_written, 2,
        "the next pass verdicts both artifacts against the amendment"
    );
}

/// FAILS IF: an anchor queued again after completing is run a second time by the same pass,
/// so a backlog the pass cannot clear spins it to the deadline.
#[sqlx::test(migrations = "../../migrations")]
async fn a_pass_runs_each_anchor_once_and_leaves_a_requeued_one_for_the_next(pool: PgPool) {
    let w = seed_world(&pool, "requeued-once-reconcile").await;
    backlog_of(&pool, &w, 1).await;
    // A backlog the pass leaves behind every time: the verdict write never lands, and the job
    // is settled as if it had, so `settle` queues it again.
    sqlx::query(
        "CREATE FUNCTION drop_every_verdict() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RETURN NULL; END $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER drop_every_verdict BEFORE INSERT ON kb_data_artifact_verdicts \
         FOR EACH ROW EXECUTE FUNCTION drop_every_verdict()",
    )
    .execute(&pool)
    .await
    .unwrap();

    // A short deadline, so the spin this test exists to catch fails it in seconds.
    let summary = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(
            std::time::Instant::now(),
            std::time::Duration::from_secs(15),
            SHAPE_RECONCILE_BATCH,
        ),
    )
    .await
    .unwrap();

    assert_eq!(summary.completed, 1, "the anchor ran once");
    assert_eq!(
        summary.requeued, 1,
        "its backlog was not empty, so it was queued again"
    );
    assert_eq!(
        summary.deferred, 1,
        "the pass claimed it again and handed it back untouched rather than running it twice"
    );
    assert_eq!(
        reconcile_job_statuses(&pool, w.context)
            .await
            .last()
            .map(String::as_str),
        Some("pending"),
        "the anchor waits for the next pass"
    );
}

/// FAILS IF: one artifact whose verdict cannot be written stops its anchor's pass, so the
/// artifacts after it are never reached; or the job is marked done with that artifact unchecked.
#[sqlx::test(migrations = "../../migrations")]
async fn an_artifact_whose_verdict_cannot_be_written_holds_back_nothing_else(pool: PgPool) {
    let w = seed_world(&pool, "poisoned-reconcile").await;
    let artifacts = backlog_of(&pool, &w, 3).await;
    let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
    ordered.sort();
    let poisoned = ordered[0];

    // A fault on the verdict write for the first artifact the pass reaches, in this test's own
    // throwaway database.
    sqlx::query(&format!(
        "CREATE FUNCTION refuse_one_verdict() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.artifact_id = '{poisoned}' THEN \
           RAISE EXCEPTION 'injected verdict fault'; END IF; RETURN NEW; END $$"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse_one_verdict BEFORE INSERT ON kb_data_artifact_verdicts \
         FOR EACH ROW EXECUTE FUNCTION refuse_one_verdict()",
    )
    .execute(&pool)
    .await
    .unwrap();

    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(
        summary.verdicts_written, 2,
        "the artifacts after the failing one are verdicted"
    );
    assert_eq!(summary.failed_artifacts, 1);
    assert_eq!(summary.failed, 1, "the job is counted as failed");
    assert_eq!(summary.completed, 0);
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["in_progress".to_owned()],
        "the job is left in flight for the reaper, not marked done"
    );
    assert_eq!(
        verdict_version(&pool, DataArtifactId::from(poisoned)).await,
        None
    );
}

/// FAILS IF: a caller's `cap` reaches the claim unbounded, or a default other than one anchor
/// at a time is used.
#[test]
fn a_claim_takes_one_anchor_by_default_and_never_more_than_the_ceiling() {
    assert_eq!(resolve_cap(None), 1);
    assert_eq!(resolve_cap(Some(4)), 4);
    assert_eq!(resolve_cap(Some(i32::MAX)), MAX_SHAPE_RECONCILE_CAP);
    assert_eq!(
        resolve_cap(Some(-3)),
        1,
        "the service holds even a caller past the door"
    );
}

/// The world's declare job, claimed, as the drain would hold it.
async fn claimed_job(pool: &PgPool) -> ClaimedAnchorJob {
    workflow_job_service::claim_anchor(
        pool,
        Persona::Shape.as_str(),
        DispatchType::ShapeReconcile.as_str(),
        1,
        DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
    )
    .await
    .unwrap()
    .into_iter()
    .next()
    .expect("the declare queued a job")
}

/// Replace the world's live shape with one that is not a field tree, as a shape stored before
/// compile refused anything else.
async fn store_uncompilable_shape(pool: &PgPool, w: &World) {
    sqlx::query(
        "UPDATE kb_data_artifact_shapes SET schema = $1 \
          WHERE home_anchor_id = $2 AND NOT is_folded",
    )
    .bind(serde_json::json!({ "anyOf": [{ "type": "object" }] }))
    .bind(w.context)
    .execute(pool)
    .await
    .unwrap();
}

/// The reasons recorded in `kb_shape_reconcile_uncheckable`.
async fn uncheckable_reasons(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT reason FROM kb_shape_reconcile_uncheckable ORDER BY reason")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// The reason an artifact is recorded unreadable under, or `None`.
async fn unreadable_reason(pool: &PgPool, artifact: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT reason FROM kb_shape_reconcile_unreadable WHERE artifact_id = $1")
        .bind(artifact)
        .fetch_optional(pool)
        .await
        .unwrap()
}

/// Fail every verdict write for `artifacts` (all of them when `None`), in this test's own
/// throwaway database.
async fn refuse_verdicts(pool: &PgPool, artifacts: Option<&[Uuid]>) {
    let condition = match artifacts {
        None => "true".to_owned(),
        Some(ids) => {
            let list: Vec<String> = ids.iter().map(|id| format!("'{id}'")).collect();
            format!("NEW.artifact_id IN ({})", list.join(","))
        }
    };
    sqlx::query(&format!(
        "CREATE FUNCTION refuse_verdicts() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF {condition} THEN RAISE EXCEPTION 'injected verdict fault'; END IF; \
         RETURN NEW; END $$"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse_verdicts BEFORE INSERT ON kb_data_artifact_verdicts \
         FOR EACH ROW EXECUTE FUNCTION refuse_verdicts()",
    )
    .execute(pool)
    .await
    .unwrap();
}

/// FAILS IF: a shape that no longer compiles fails its anchor, gains verdicts, or leaves its
/// artifacts in the backlog, so every pass compiles it again and requeues the anchor.
#[sqlx::test(migrations = "../../migrations")]
async fn a_shape_that_does_not_compile_is_recorded_once_and_its_artifacts_leave_the_backlog(
    pool: PgPool,
) {
    let w = seed_world(&pool, "uncompilable-reconcile").await;
    let artifacts = backlog_of(&pool, &w, 2).await;
    store_uncompilable_shape(&pool, &w).await;

    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(summary.failed, 0, "the anchor's pass does not fail");
    assert_eq!(summary.completed, 1);
    assert_eq!(summary.requeued, 0, "nothing is left to queue again");
    assert_eq!(summary.verdicts_written, 0);
    assert_eq!(summary.shapes_uncheckable, 1);
    assert_eq!(summary.uncheckable_artifacts, 2);
    assert_eq!(
        uncheckable_reasons(&pool).await,
        vec!["schema_uncompilable".to_owned()]
    );
    for artifact in &artifacts {
        assert_eq!(verdict_version(&pool, *artifact).await, None);
    }
    let anchor = HomeAnchor::Context(ContextId::from(w.context));
    assert!(
        backlog_page(&pool, anchor, Uuid::nil(), 10)
            .await
            .unwrap()
            .is_empty(),
        "an artifact under a recorded shape is not backlog"
    );
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["done".to_owned()]
    );
}

/// FAILS IF: a record outlives the shape it names, so an amendment of an uncompilable shape is
/// never tried.
#[sqlx::test(migrations = "../../migrations")]
async fn an_amended_shape_is_tried_afresh(pool: PgPool) {
    let w = seed_world(&pool, "amended-uncompilable-reconcile").await;
    backlog_of(&pool, &w, 2).await;
    store_uncompilable_shape(&pool, &w).await;
    dispatch_tick(&pool, None).await.unwrap();

    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind: "measurement",
            kind_owner: None,
            schema: &string_value_schema(),
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();
    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(
        summary.verdicts_written, 2,
        "both artifacts are verdicted against the amendment"
    );
    assert_eq!(summary.shapes_uncheckable, 0);
}

/// FAILS IF: a batch is bounded by the bytes committed rather than the bytes Postgres returns.
/// `jsonb` prints `1e308` as 309 digits, so two such artifacts that fit one batch by their
/// committed size do not fit it by what is read.
#[sqlx::test(migrations = "../../migrations")]
async fn content_is_batched_by_what_postgres_returns_not_what_was_committed(pool: PgPool) {
    let w = seed_world(&pool, "returned-bytes-reconcile").await;
    let home = AnchorRef::context(ContextId::from(w.context));
    let content = serde_json::json!({ "value": 1e308 });
    for i in 0..2 {
        let resource = make_resource(
            &pool,
            ProfileId::from(w.owner),
            EntityId::from(w.emitter),
            home,
            &format!("wide-{i}"),
        )
        .await;
        commit_artifact(&pool, resource, "measurement", &content, w.emitter).await;
    }
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home,
            kind: "measurement",
            kind_owner: None,
            schema: &string_value_schema(),
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();
    let bytes: i64 = 100;
    let (committed, returned): (i64, i64) = sqlx::query_as(
        "SELECT max(a.content_bytes), max(octet_length(c.content::text))::bigint \
           FROM kb_data_artifacts a JOIN kb_data_artifact_content c ON c.artifact_id = a.id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        2 * committed <= bytes && returned > bytes,
        "precondition: both fit the bound as committed ({committed} each), and neither as \
         returned ({returned} each)"
    );

    let summary = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(
            std::time::Instant::now(),
            ROOMY,
            BatchBound {
                rows: 100,
                bytes,
                max_one: i64::MAX,
            },
        ),
    )
    .await
    .unwrap();

    assert_eq!(summary.verdicts_written, 2);
    assert_eq!(
        summary.batches_read, 2,
        "each artifact returns more than the bound, so each is a batch alone"
    );
}

/// FAILS IF: an artifact whose returned content is past the per-artifact ceiling is read, is
/// not recorded with its reason, stops the artifacts after it, or keeps its anchor's job alive.
#[sqlx::test(migrations = "../../migrations")]
async fn an_artifact_past_the_one_artifact_ceiling_is_recorded_unread_and_holds_back_nothing_else(
    pool: PgPool,
) {
    let w = seed_world(&pool, "ceiling-reconcile").await;
    let home = AnchorRef::context(ContextId::from(w.context));
    let mut wide = None;
    for (i, value) in ["42".to_owned(), "x".repeat(1_000), "43".to_owned()]
        .into_iter()
        .enumerate()
    {
        let resource = make_resource(
            &pool,
            ProfileId::from(w.owner),
            EntityId::from(w.emitter),
            home,
            &format!("ceiling-{i}"),
        )
        .await;
        let long = value.len() > 2;
        let (artifact, _) = commit_artifact(
            &pool,
            resource,
            "measurement",
            &serde_json::json!({ "value": value }),
            w.emitter,
        )
        .await;
        if long {
            wide = Some(artifact);
        }
    }
    declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home,
            kind: "measurement",
            kind_owner: None,
            schema: &string_value_schema(),
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(w.owner),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    let summary = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(
            std::time::Instant::now(),
            ROOMY,
            BatchBound {
                rows: 100,
                bytes: i64::MAX,
                max_one: 100,
            },
        ),
    )
    .await
    .unwrap();

    let wide = wide.expect("one artifact is wide");
    assert_eq!(summary.verdicts_written, 2, "the other two are verdicted");
    assert_eq!(summary.unreadable_artifacts, 1);
    assert_eq!(summary.failed_artifacts, 0, "a record is not a failure");
    assert_eq!(summary.completed, 1, "the job completes");
    assert_eq!(verdict_version(&pool, wide).await, None);
    assert_eq!(
        unreadable_reason(&pool, wide.uuid()).await.as_deref(),
        Some("content_too_large")
    );
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["done".to_owned()],
        "the recorded artifact is not backlog, so nothing is queued again"
    );
}

/// FAILS IF: a run of artifacts that cannot be written is walked to the end of the backlog, or
/// stops the pass without leaving the job for the reaper.
#[sqlx::test(migrations = "../../migrations")]
async fn a_run_of_failed_artifacts_stops_the_pass_and_leaves_the_job_for_the_reaper(pool: PgPool) {
    let w = seed_world(&pool, "breaker-reconcile").await;
    backlog_of(&pool, &w, MAX_CONSECUTIVE_FAILED_ARTIFACTS as usize + 2).await;
    refuse_verdicts(&pool, None).await;

    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(
        summary.failed_artifacts, MAX_CONSECUTIVE_FAILED_ARTIFACTS,
        "the pass stops at the breaker, not at the end of the backlog"
    );
    assert_eq!(summary.verdicts_written, 0);
    assert_eq!(summary.failed, 1);
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["in_progress".to_owned()],
        "the job is left in flight for the reaper, which counts the attempt"
    );
}

/// FAILS IF: a pass cut off having settled nothing hands its job back, which costs no attempt,
/// so an anchor whose failures use up its time is handed back every pass and never dies.
#[sqlx::test(migrations = "../../migrations")]
async fn a_pass_cut_off_having_settled_nothing_leaves_its_job_for_the_reaper(pool: PgPool) {
    let w = seed_world(&pool, "cut-off-failed-reconcile").await;
    let artifacts = backlog_of(&pool, &w, 3).await;
    let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
    ordered.sort();
    refuse_verdicts(&pool, Some(&ordered[..1])).await;
    let job = claimed_job(&pool).await;
    let pace = Pace::new(
        std::time::Instant::now(),
        std::time::Duration::ZERO,
        SHAPE_RECONCILE_BATCH,
    );

    let ran = run_reconcile_job(&pool, &job, None, false, pace).await;

    let ReconcileJobResult::Ran { pass, settled } = ran else {
        panic!("the job ran a pass: {ran:?}");
    };
    assert_eq!(pass.failed_artifacts, 1);
    assert!(!pass.made_progress());
    assert_eq!(pass.stop, Stop::Deadline);
    assert_eq!(settled, Settled::LeftInFlight);
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["in_progress".to_owned()],
        "left for the reaper, which counts the attempt"
    );
}

/// FAILS IF: a stop that comes after a failed measurement ends the pass without settling what
/// it had measured, so the pass settles nothing and its job is left for the reaper.
#[sqlx::test(migrations = "../../migrations")]
async fn a_stop_after_a_failed_measurement_still_settles_what_was_measured(pool: PgPool) {
    let w = seed_world(&pool, "flush-on-stop-reconcile").await;
    let artifacts = backlog_of(&pool, &w, 3).await;
    let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
    ordered.sort();
    poison_measurement(&pool, &ordered[1..2], "XX000").await;
    let job = claimed_job(&pool).await;
    let pace = Pace::new(
        std::time::Instant::now(),
        std::time::Duration::ZERO,
        SHAPE_RECONCILE_BATCH,
    );

    let ran = run_reconcile_job(&pool, &job, None, false, pace).await;

    let ReconcileJobResult::Ran { pass, settled } = ran else {
        panic!("the job ran a pass: {ran:?}");
    };
    assert_eq!(pass.failed_artifacts, 1);
    assert_eq!(
        pass.verdicts_written, 1,
        "the artifact measured before the failure is settled"
    );
    assert_eq!(pass.stop, Stop::Deadline);
    assert_eq!(settled, Settled::HandedBack);
}

/// FAILS IF: a measurement breaks a run of failed verdict writes, so a database refusing every
/// write trips the breaker only when eight artifacts share one batch.
#[sqlx::test(migrations = "../../migrations")]
async fn failed_verdict_writes_trip_the_breaker_across_batches(pool: PgPool) {
    let w = seed_world(&pool, "breaker-batches-reconcile").await;
    backlog_of(&pool, &w, MAX_CONSECUTIVE_FAILED_ARTIFACTS as usize + 2).await;
    refuse_verdicts(&pool, None).await;
    let one_a_batch = BatchBound {
        rows: 100,
        bytes: 1,
        max_one: i64::MAX,
    };

    let summary = dispatch_tick_inner(
        &pool,
        None,
        Pace::new(std::time::Instant::now(), ROOMY, one_a_batch),
    )
    .await
    .unwrap();

    assert_eq!(
        summary.failed_artifacts, MAX_CONSECUTIVE_FAILED_ARTIFACTS,
        "the breaker stops the pass at the eighth write, each in its own batch"
    );
    assert_eq!(summary.failed, 1);
}

/// FAILS IF: a hand-back whose successor cannot be queued still marks the job done, leaving the
/// anchor with no job at all.
#[sqlx::test(migrations = "../../migrations")]
async fn a_hand_back_that_cannot_queue_its_successor_leaves_the_job_in_flight(pool: PgPool) {
    let w = seed_world(&pool, "atomic-hand-back").await;
    backlog_of(&pool, &w, 1).await;
    let job = claimed_job(&pool).await;
    sqlx::query(
        "CREATE FUNCTION refuse_jobs() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected enqueue fault'; END $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse_jobs BEFORE INSERT ON kb_workflow_jobs \
         FOR EACH ROW EXECUTE FUNCTION refuse_jobs()",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(hand_back(&pool, &job).await.is_err());
    assert_eq!(
        reconcile_job_statuses(&pool, w.context).await,
        vec!["in_progress".to_owned()],
        "the completion rolled back with the enqueue"
    );
}

/// FAILS IF: the deadline override is read past its ceiling, the ceiling leaves no room under
/// the `api/internal` function's `maxDuration` to hand a job back, or a value that is not a
/// positive whole number of seconds is used.
#[test]
fn the_deadline_override_is_held_under_the_function_max_duration() {
    let secs = |raw| deadline_from(raw).as_secs();
    assert_eq!(secs(None), DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS);
    assert_eq!(secs(Some(" 60 ")), 60);
    assert_eq!(secs(Some("0")), DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS);
    assert_eq!(secs(Some("soon")), DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS);
    assert_eq!(secs(Some("100000")), MAX_SHAPE_RECONCILE_DEADLINE_SECONDS);

    let vercel: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../vercel.json")).unwrap();
    let max_duration = vercel["functions"]["api/internal.rs"]["maxDuration"]
        .as_u64()
        .expect("vercel.json names api/internal.rs's maxDuration");
    assert!(
        MAX_SHAPE_RECONCILE_DEADLINE_SECONDS + 30 <= max_duration,
        "the ceiling leaves 30 s under maxDuration ({max_duration}) to hand a job back"
    );
    assert!(
        max_duration < u64::try_from(DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS).unwrap(),
        "a killed function's job is reaped only after its lease"
    );
}

/// FAILS IF: a batch takes an artifact that crosses its byte or row bound, or ever refuses
/// the first artifact.
#[test]
fn an_artifact_joins_a_batch_only_within_its_bounds_and_the_first_always() {
    let bound = BatchBound {
        rows: 3,
        bytes: 40,
        max_one: i64::MAX,
    };
    assert!(joins_batch(0, 0, 500, bound), "the first always");
    assert!(joins_batch(2, 30, 10, bound), "exactly at the byte bound");
    assert!(!joins_batch(2, 30, 11, bound), "one byte past it");
    assert!(!joins_batch(3, 3, 1, bound), "past the row bound");
}

/// The contexts of the artifacts the drain verdicted, in the order it wrote them.
async fn verdict_contexts_in_order(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT h.anchor_id FROM kb_data_artifact_verdicts v \
           JOIN kb_data_artifacts a ON a.id = v.artifact_id \
           JOIN kb_resource_homes h ON h.resource_id = a.resource_id \
          ORDER BY v.checked_at",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// FAILS IF: an anchor holds the pass until its backlog is exhausted, so an anchor queued
/// behind it waits; or one cut off by its slice is not run again when the pass has time.
#[sqlx::test(migrations = "../../migrations")]
async fn anchors_take_turns_by_slice_and_each_finishes_in_one_pass(pool: PgPool) {
    let first = seed_world(&pool, "slice-first").await;
    backlog_of(&pool, &first, 3).await;
    let second = World {
        context: insert_context(&pool, first.owner, "slice-second").await,
        ..first
    };
    backlog_of(&pool, &second, 3).await;
    let pace = Pace {
        slice: std::time::Duration::ZERO,
        ..Pace::new(std::time::Instant::now(), ROOMY, SHAPE_RECONCILE_BATCH)
    };

    let summary = dispatch_tick_inner(&pool, None, pace).await.unwrap();

    assert_eq!(summary.verdicts_written, 6, "both backlogs in one pass");
    assert_eq!(summary.completed, 2);
    let order = verdict_contexts_in_order(&pool).await;
    assert_ne!(
        order[0], order[1],
        "the second anchor's first verdict comes before the first anchor's second: {order:?}"
    );
}

/// FAILS IF: the drain reads the content of a resource the erasure estate has fenced
/// (`is_active` false), or leaves it out once it is active again.
#[sqlx::test(migrations = "../../migrations")]
async fn a_fenced_resources_artifact_is_not_backlog_until_it_is_active_again(pool: PgPool) {
    let w = seed_world(&pool, "fenced-reconcile").await;
    let artifacts = backlog_of(&pool, &w, 2).await;
    let fenced = artifacts[0];
    sqlx::query(
        "UPDATE kb_resources SET is_active = false \
          WHERE id = (SELECT resource_id FROM kb_data_artifacts WHERE id = $1)",
    )
    .bind(fenced.uuid())
    .execute(&pool)
    .await
    .unwrap();

    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(summary.verdicts_written, 1, "only the active resource's");
    assert_eq!(verdict_version(&pool, fenced).await, None);
    sqlx::query(
        "UPDATE kb_resources SET is_active = true \
          WHERE id = (SELECT resource_id FROM kb_data_artifacts WHERE id = $1)",
    )
    .bind(fenced.uuid())
    .execute(&pool)
    .await
    .unwrap();
    let anchor = HomeAnchor::Context(ContextId::from(w.context));
    let backlog: Vec<Uuid> = backlog_page(&pool, anchor, Uuid::nil(), 10)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.artifact_id)
        .collect();
    assert_eq!(
        backlog,
        vec![fenced.uuid()],
        "active again, it is backlog again"
    );
}

/// Make measuring each of `poisoned`'s content fail with SQLSTATE `code`, in this test's own
/// throwaway database, by replacing the measure the drain calls.
async fn poison_measurement(pool: &PgPool, poisoned: &[Uuid], code: &str) {
    let list: Vec<String> = poisoned.iter().map(|id| format!("'{id}'")).collect();
    sqlx::query(&format!(
        "CREATE OR REPLACE FUNCTION data_artifact_content_returned_bytes(p_artifact_id uuid) \
         RETURNS bigint LANGUAGE plpgsql AS $$ \
         BEGIN IF p_artifact_id IN ({}) THEN \
           RAISE EXCEPTION 'injected measurement fault' USING ERRCODE = '{code}'; \
         END IF; \
         RETURN (SELECT octet_length(content::text) FROM kb_data_artifact_content \
                  WHERE artifact_id = p_artifact_id); END $$",
        list.join(",")
    ))
    .execute(pool)
    .await
    .unwrap();
}

/// FAILS IF: a measured artifact does not break a run of failures, so failures scattered among
/// healthy artifacts trip the breaker and stop the pass before the healthy ones are verdicted.
#[sqlx::test(migrations = "../../migrations")]
async fn failures_scattered_among_healthy_artifacts_never_trip_the_breaker(pool: PgPool) {
    let w = seed_world(&pool, "scattered-reconcile").await;
    let n = 2 * MAX_CONSECUTIVE_FAILED_ARTIFACTS as usize + 2;
    let artifacts = backlog_of(&pool, &w, n).await;
    let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
    ordered.sort();
    let poisoned: Vec<Uuid> = ordered.iter().copied().step_by(2).collect();
    poison_measurement(&pool, &poisoned, "XX000").await;

    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(
        summary.failed_artifacts as usize,
        poisoned.len(),
        "every poisoned artifact failed, and the pass reached them all"
    );
    assert_eq!(summary.verdicts_written as usize, n - poisoned.len());
    assert_eq!(
        summary.failed, 1,
        "the job is left in flight for the failures"
    );
}

/// FAILS IF: content Postgres refuses to render (a program-limit error) is counted as a
/// failure that may pass, so its anchor's job is retried until it dies, rather than recorded.
#[sqlx::test(migrations = "../../migrations")]
async fn content_postgres_cannot_render_is_recorded_unrenderable(pool: PgPool) {
    let w = seed_world(&pool, "unrenderable-reconcile").await;
    let artifacts = backlog_of(&pool, &w, 2).await;
    let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
    ordered.sort();
    poison_measurement(&pool, &ordered[..1], "54000").await;

    let summary = dispatch_tick(&pool, None).await.unwrap();

    assert_eq!(summary.unreadable_artifacts, 1);
    assert_eq!(summary.failed_artifacts, 0);
    assert_eq!(summary.verdicts_written, 1);
    assert_eq!(summary.completed, 1);
    assert_eq!(
        unreadable_reason(&pool, ordered[0]).await.as_deref(),
        Some("content_unrenderable")
    );
}

/// FAILS IF: a real program-limit refusal is not read as one, or another error is.
#[sqlx::test(migrations = "../../migrations")]
async fn a_program_limit_refusal_is_told_apart_from_other_errors(pool: PgPool) {
    let limit = sqlx::query("SELECT repeat('x', 1100000000)")
        .execute(&pool)
        .await
        .expect_err("a 1.1 GB string is past Postgres's limit");
    assert!(is_program_limit(&limit), "{limit}");
    let other = sqlx::query("SELECT 1 / 0")
        .execute(&pool)
        .await
        .expect_err("division by zero");
    assert!(!is_program_limit(&other), "{other}");
}
