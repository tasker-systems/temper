//! Shape reconciliation on the anchor-scoped job queue (spec §7.3, plan Task 6).
//!
//! Declaring a shape enqueues a reconcile job on the existing `kb_workflow_jobs` anchor queue —
//! no new queue infrastructure. The worker claims jobs and verdicts pre-existing artifacts by
//! running `jsonschema` validation against the shape in force.
//!
//! ## Authority gate
//!
//! The authority gate lives HERE, not in Beat E: spec §5 rules that declaring a shape requires
//! authority over its home (`context_authorable_by_profile` / `cogmap_authorable_by_profile`), and
//! Beats A–D otherwise ship a write path with no authorization check. The gate calls the
//! predicates, never restates them — the same two-arm branch `can_modify_resource` uses
//! (`migrations/20260804000020:102-108`).

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::workflow_job_service;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, EntityId, ProfileId, ShapeId};
use temper_core::types::workflow_job::{AnchorJobPayload, DispatchType, Persona};
use temper_substrate::payloads::{AnchorRef, EnforcementMode, KindOwner};
use temper_substrate::writes::{self, DeclareShapeParams};

/// Lease for a claimed shape-reconcile job. Same reasoning as the region lease: MUST exceed the
/// Vercel function timeout (300s) so a genuinely-running reconciliation never looks dead to the
/// reaper.
const DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS: i32 = 600;

/// How many anchor-keyed jobs one reconcile tick claims.
const DEFAULT_SHAPE_RECONCILE_CAP: i32 = 10;

/// Parameters for [`declare_shape`] — the service-layer write that gates on authority, fires the
/// substrate declare, and enqueues a reconcile job.
#[derive(Debug)]
pub struct DeclareShapeServiceParams<'a> {
    pub home: AnchorRef,
    pub kind: &'a str,
    pub kind_owner: Option<KindOwner>,
    pub schema: &'a serde_json::Value,
    pub enforcement: EnforcementMode,
    pub principal: ProfileId,
    pub emitter: EntityId,
}

/// Declare a shape: authority gate → substrate declare → enqueue reconcile job.
///
/// The authority gate is the two-arm branch from spec §5: `context_authorable_by_profile` for a
/// context-homed shape, `cogmap_authorable_by_profile` for a cogmap-homed shape. It calls the
/// predicates, never restates them. A principal who cannot author the home is refused with
/// `Forbidden` — the same denial `DbBackend::check_context_authorable` /
/// `DbBackend::check_cogmap_authorable` use.
///
/// After the declare succeeds, a reconcile job is enqueued on the anchor queue. The single-flight
/// index collapses N declarations into one job — the second enqueue returns `None`, not an error.
pub async fn declare_shape(pool: &PgPool, p: DeclareShapeServiceParams<'_>) -> ApiResult<ShapeId> {
    // Authority gate — the two-arm branch from spec §5. Call the predicates, never restate them.
    check_home_authorable(pool, p.principal, &p.home).await?;

    // Substrate declare (fires the event, projects the row).
    let shape_id = writes::declare_shape(
        pool,
        DeclareShapeParams {
            home: p.home,
            kind: p.kind,
            kind_owner: p.kind_owner,
            schema: p.schema,
            enforcement: p.enforcement,
            emitter: p.emitter,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Enqueue a reconcile job on the anchor queue. The single-flight index collapses N
    // declarations into one job; the second enqueue returns None, not an error.
    let anchor = anchor_from_ref(&p.home);
    workflow_job_service::enqueue_anchor(
        pool,
        anchor,
        Persona::Shape.as_str(),
        DispatchType::ShapeReconcile.as_str(),
        AnchorJobPayload {
            emitter: p.emitter.uuid(),
        },
    )
    .await?;

    Ok(shape_id)
}

/// Run one shape-reconciliation pass: claim anchor-keyed jobs, verdict pre-existing artifacts by
/// running `jsonschema` validation against the shape in force, complete each job. Returns the
/// count of verdicts written.
///
/// A tick that claims zero jobs must not be mistakable for a successful sweep — the caller should
/// assert on verdicts written, not on the tick returning `Ok`. The returned count is the work
/// product, not the claim count.
pub async fn reconcile_anchor(pool: &PgPool, _anchor: HomeAnchor) -> ApiResult<usize> {
    let persona = Persona::Shape.as_str();
    let dispatch = DispatchType::ShapeReconcile.as_str();

    // Reap stale leases before claiming, mirroring the region and embed ticks.
    workflow_job_service::reap(pool, "shape reconcile lease expired").await?;

    let claimed = workflow_job_service::claim_anchor(
        pool,
        persona,
        dispatch,
        DEFAULT_SHAPE_RECONCILE_CAP,
        DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
    )
    .await?;

    if claimed.is_empty() {
        return Ok(0);
    }

    let mut verdicts_written = 0usize;
    for job in claimed {
        match reconcile_one_anchor(pool, job.anchor).await {
            Ok(count) => {
                verdicts_written += count;
                workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await?;
            }
            Err(e) => {
                // Leave the job in_progress; the reaper retries it. One bad anchor never aborts
                // the pass — same posture as the region drain.
                tracing::warn!(
                    error = %e,
                    anchor = %job.anchor.uuid(),
                    "shape reconciliation failed for one anchor; left in-flight for the reaper"
                );
            }
        }
    }

    Ok(verdicts_written)
}

/// Reconcile all unchecked artifacts for one anchor: find artifacts whose shape in force is homed
/// here and whose staleness triple does not match, validate each against the shape's JSON Schema,
/// and upsert the verdict. Returns the count of verdicts written.
async fn reconcile_one_anchor(pool: &PgPool, anchor: HomeAnchor) -> ApiResult<usize> {
    // Find the backlog: live artifacts whose shape in force is homed in this anchor and whose
    // staleness triple (shape_id, shape_version, content_hash) does not match any stored verdict.
    //
    // Uses _data_artifact_shape_in_force (which calls _data_artifact_anchor and
    // _data_artifact_kind_owner) rather than re-deriving the home — the cogmap tiebreak is
    // load-bearing (20260820000020:186-188).
    let rows = sqlx::query!(
        r#"
        SELECT a.id              AS "artifact_id!: Uuid",
               a.content_hash    AS "content_hash!: String",
               c.content         AS "content!: serde_json::Value",
               s.shape_id        AS "shape_id!: Uuid",
               s.shape_version   AS "shape_version!: i32",
               s.schema          AS "schema!: serde_json::Value"
          FROM kb_data_artifacts a
          LEFT JOIN kb_data_artifact_content c ON c.artifact_id = a.id
          CROSS JOIN LATERAL _data_artifact_shape_in_force(
              a.resource_id, a.kind_owner_table, a.kind_owner_id, a.artifact_kind
          ) s
          JOIN kb_data_artifact_shapes sh ON sh.id = s.shape_id
            AND sh.home_anchor_table = $1
            AND sh.home_anchor_id    = $2
         WHERE NOT a.is_folded
           AND NOT EXISTS (
               SELECT 1 FROM kb_data_artifact_verdicts v
                WHERE v.artifact_id   = a.id
                  AND v.shape_id      = s.shape_id
                  AND v.shape_version = s.shape_version
                  AND v.content_hash  = a.content_hash
           )
        "#,
        anchor.table(),
        anchor.uuid(),
    )
    .fetch_all(pool)
    .await?;

    let mut count = 0usize;
    for row in rows {
        let validator = jsonschema::validator_for(&row.schema)
            .map_err(|e| ApiError::Internal(format!("failed to compile shape schema: {e}")))?;
        let errors: Vec<_> = validator.iter_errors(&row.content).collect();
        let satisfied = errors.is_empty();

        let detail = if satisfied {
            None
        } else {
            Some(serde_json::Value::Array(
                errors
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "path": e.instance_path().to_string(),
                            "message": e.to_string(),
                        })
                    })
                    .collect(),
            ))
        };

        sqlx::query!(
            "SELECT data_artifact_verdict_upsert($1,$2,$3,$4,$5,$6)",
            row.artifact_id,
            row.shape_id,
            row.shape_version,
            row.content_hash,
            satisfied,
            detail,
        )
        .execute(pool)
        .await?;

        count += 1;
    }

    Ok(count)
}

/// The authority gate: call `context_authorable_by_profile` / `cogmap_authorable_by_profile` for
/// the home anchor, never restate them. Returns `Forbidden` on denial — the same denial
/// `DbBackend::check_context_authorable` / `DbBackend::check_cogmap_authorable` use.
async fn check_home_authorable(
    pool: &PgPool,
    principal: ProfileId,
    home: &AnchorRef,
) -> ApiResult<()> {
    let can = match home.table {
        temper_substrate::payloads::AnchorTable::Contexts => {
            sqlx::query_scalar!(
                "SELECT context_authorable_by_profile($1, $2)",
                *principal,
                home.id,
            )
            .fetch_one(pool)
            .await?
        }
        temper_substrate::payloads::AnchorTable::Cogmaps => {
            sqlx::query_scalar!(
                "SELECT cogmap_authorable_by_profile($1, $2)",
                *principal,
                home.id,
            )
            .fetch_one(pool)
            .await?
        }
        _ => {
            return Err(ApiError::BadRequest(
                "shape home must be a context or cogmap".to_string(),
            ))
        }
    };

    if can.unwrap_or(false) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Convert an `AnchorRef` to a `HomeAnchor`. The `AnchorRef` carries the raw `(table, id)` pair;
/// `HomeAnchor` is the typed closed two-variant enum.
fn anchor_from_ref(r: &AnchorRef) -> HomeAnchor {
    match r.table {
        temper_substrate::payloads::AnchorTable::Contexts => {
            HomeAnchor::Context(ContextId::from(r.id))
        }
        temper_substrate::payloads::AnchorTable::Cogmaps => {
            HomeAnchor::Cogmap(CogmapId::from(r.id))
        }
        other => panic!("shape home anchor must be context or cogmap, got {other:?}"),
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_substrate::events::{fire, EventContext, Fired, SeedAction};
    use temper_substrate::ids::{DataArtifactId, ResourceId};
    use temper_substrate::payloads::{ArtifactIntent, ShapeState};
    use temper_substrate::scenario::bootseed;
    use temper_substrate::writes::{self, CreateParams};

    // ── seeding helpers ──────────────────────────────────────────────────────

    async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_entity(pool: &PgPool, profile: Uuid, name: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(profile)
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
        )
        .bind(owner)
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
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
        let count = reconcile_anchor(&pool, HomeAnchor::Context(ContextId::from(w.context)))
            .await
            .unwrap();

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
        let count = reconcile_anchor(&pool, HomeAnchor::Context(ContextId::from(w.context)))
            .await
            .unwrap();

        assert_eq!(
            count, 0,
            "a tick with no work must write zero verdicts — not be mistakable for a successful sweep"
        );

        // No verdict rows exist.
        let verdict_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_verdicts")
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
}
