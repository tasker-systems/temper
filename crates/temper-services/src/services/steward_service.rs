//! Read path for the team-self-cognition steward's ingest trigger (T4a).
//!
//! Service-direct (the read-path convention): the surface passes a resolved cogmap id + optional
//! threshold; this gates on `anchor_readable_by_profile` and returns the [`IngestDelta`]. The write
//! side (advancing the watermark) routes through the `Backend` trait / `DbBackend`, not here.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::steward::{DriftSweepRow, IngestDelta, DEFAULT_STEWARD_INGEST_THRESHOLD};

/// Compute the ingest delta for a team-self-cognition cogmap: how many new resources + events have
/// landed in the team's contexts since the cogmap's watermark, and whether that clears `threshold`.
///
/// Auth: the caller must be able to READ the cogmap (`anchor_readable_by_profile`). A cogmap the
/// caller cannot see is reported as `NotFound` (show-deny → 404, never leaking existence).
pub async fn ingest_delta(
    pool: &PgPool,
    principal: ProfileId,
    cogmap_id: CogmapId,
    threshold: Option<i64>,
) -> ApiResult<IngestDelta> {
    // One query does the read-gate AND both stored cursors: an absent row means the cogmap does not
    // exist OR the caller cannot read it — both surface as NotFound. Both columns are nullable (NULL
    // watermark = never run; NULL fingerprint = never snapshotted), and they are read together on
    // purpose — the delta compares against BOTH, so a second round trip could only serve a torn pair.
    let cursors = sqlx::query!(
        r#"
        SELECT steward_watermark_event_id  AS "watermark: Uuid",
               steward_boundary_fingerprint AS "stored_fingerprint: String"
          FROM kb_cogmaps
         WHERE id = $1
           AND anchor_readable_by_profile($2, 'kb_cogmaps', $1)
        "#,
        *cogmap_id,
        *principal,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("cognitive map not found or not readable".to_string()))?;
    let watermark = cursors.watermark;

    let row = sqlx::query!(
        r#"
        SELECT new_resources        AS "new_resources!: i64",
               new_events           AS "new_events!: i64",
               max_event_id         AS "max_event_id: Uuid",
               boundary_fingerprint AS "boundary_fingerprint: String",
               boundary_moved       AS "boundary_moved!: bool"
          FROM steward_ingest_delta($1, $2, $3)
        "#,
        *cogmap_id,
        watermark,
        cursors.stored_fingerprint,
    )
    .fetch_one(pool)
    .await?;

    let threshold = threshold.unwrap_or(DEFAULT_STEWARD_INGEST_THRESHOLD);
    Ok(IngestDelta {
        cogmap_id: *cogmap_id,
        watermark,
        new_resources: row.new_resources,
        new_events: row.new_events,
        max_event_id: row.max_event_id,
        threshold,
        // Two arms, OR'd: the counted one, and the boundary one. The boundary arm exists because the
        // counts are taken INSIDE a scope that can itself move silently (a `kb_team_contexts` share
        // emits no event), so a count-only gate under-triggers precisely when the most material
        // changed.
        exceeds_threshold: row.new_resources >= threshold || row.boundary_moved,
        boundary_fingerprint: row.boundary_fingerprint,
        boundary_moved: row.boundary_moved,
    })
}

/// Sweep all team-joined cogmaps the principal can read, returning those whose ingest delta clears
/// `threshold` **or whose change-detection boundary moved**, most-drifted-first. The privileged case
/// (the steward app-principal) simply has broad read; the gate is the same
/// `anchor_readable_by_profile` every read uses — not a bypass.
///
/// The boundary arm is the same OR that `ingest_delta`'s `exceeds_threshold` carries, and it lives in
/// `steward_drift_sweep`'s own WHERE rather than being re-derived here: the cron path and the
/// per-cogmap path must not be able to disagree about what "drifted" means.
pub async fn drift_sweep(
    pool: &PgPool,
    principal: ProfileId,
    threshold: Option<i64>,
) -> ApiResult<Vec<DriftSweepRow>> {
    let threshold = threshold.unwrap_or(DEFAULT_STEWARD_INGEST_THRESHOLD);
    let rows = sqlx::query!(
        r#"
        SELECT cogmap_id      AS "cogmap_id!: Uuid",
               watermark      AS "watermark: Uuid",
               new_resources  AS "new_resources!: i64",
               new_events     AS "new_events!: i64",
               boundary_moved AS "boundary_moved!: bool"
          FROM steward_drift_sweep($1, $2)
        "#,
        *principal,
        threshold,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DriftSweepRow {
            cogmap_id: r.cogmap_id,
            watermark: r.watermark,
            new_resources: r.new_resources,
            new_events: r.new_events,
            boundary_moved: r.boundary_moved,
        })
        .collect())
}

/// All team-joined cogmaps the principal can AUTHOR (the materialize fan-out candidate set).
///
/// Authorable, not merely readable: this feeds the hourly materialize cron, whose
/// `materialize_on_threshold` gate is `cogmap_authorable_by_profile` — an explicit write grant. A
/// readable-but-unauthorable map handed out here is a guaranteed 403, and the cron fans out with
/// `Promise.all` and throws on any non-ok, so ONE such map fails the entire tick. The L0 kernel is
/// exactly that map for every principal (root-team readable, authorable by nobody by design), which
/// is why it has never once materialized.
///
/// The auditor's candidate set is a different question with a different answer and deliberately still
/// calls `steward_candidate_cogmaps` — see `20260805000010`'s header.
pub async fn candidate_cogmaps(pool: &PgPool, principal: ProfileId) -> ApiResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar!(
        r#"SELECT cogmap_id AS "id!: Uuid" FROM steward_authorable_cogmaps($1)"#,
        *principal,
    )
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use crate::backend::DbBackend;
    use sqlx::PgPool;
    use temper_core::error::TemperError;
    use temper_workflow::operations::{AdvanceStewardWatermark, Backend, Surface};

    /// A minimal team-self-cognition graph: a member profile (in the team joined to the cogmap), an
    /// outsider (no access), the cogmap, the team-owned context (the ingest source), and an unrelated
    /// context (to prove scoping). Emitter entity for synthesizing events.
    struct Seeded {
        member: Uuid,
        outsider: Uuid,
        /// The team joined to the cogmap — the thing the change-detection boundary is drawn around.
        /// Exposed so the boundary tests can move a context INTO it (share or ownership transfer).
        team: Uuid,
        cogmap: Uuid,
        ctx: Uuid,
        other_ctx: Uuid,
        entity: Uuid,
    }

    async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn seed(pool: &PgPool) -> Seeded {
        let member = insert_profile(pool, "member").await;
        let outsider = insert_profile(pool, "outsider").await;

        let team: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('team', 'Team') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(team)
        .bind(member)
        .execute(pool)
        .await
        .unwrap();

        let telos: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let cogmap: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('map', $1) RETURNING id",
        )
        .bind(telos)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
            .bind(cogmap)
            .bind(team)
            .execute(pool)
            .await
            .unwrap();
        // NOTE: the member gets team membership and NO write grant. Deliberate — this module has
        // tests on both sides of that grant (`advance_requires_cogmap_write_grant` needs it withheld;
        // the sweep tests need it present since `20260805000010`), so the fixture must not pre-decide
        // it. Tests needing authorship call `grant_cogmap_write` and say why.

        let ctx: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_teams', $1, 'building', 'Building') RETURNING id",
        )
        .bind(team)
        .fetch_one(pool)
        .await
        .unwrap();
        // A context the team does NOT own (owned by the outsider) — its events must be excluded.
        let other_ctx: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'elsewhere', 'Elsewhere') RETURNING id",
        )
        .bind(outsider)
        .fetch_one(pool)
        .await
        .unwrap();

        let entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'e') RETURNING id",
        )
        .bind(member)
        .fetch_one(pool)
        .await
        .unwrap();

        Seeded {
            member,
            outsider,
            team,
            cogmap,
            ctx,
            other_ctx,
            entity,
        }
    }

    /// Append a synthetic event of `type_name` anchored to a context (append-only allows INSERT).
    async fn add_event(pool: &PgPool, entity: Uuid, type_name: &str, ctx: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = $1), $2, 'kb_contexts', $3) RETURNING id",
        )
        .bind(type_name)
        .bind(entity)
        .bind(ctx)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn grant_cogmap_write(pool: &PgPool, cogmap: Uuid, profile: Uuid) {
        // Write implies read (kb_access_grants monotonic check): grant both.
        sqlx::query(
            "INSERT INTO kb_access_grants \
               (subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
             VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2)",
        )
        .bind(cogmap)
        .bind(profile)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Put the cogmap in the state a *completed* run leaves behind: watermark at `watermark_event`,
    /// boundary fingerprint at the boundary's current shape. Prior state only: the production write of
    /// these two columns is pinned by the two tests that go through
    /// `DbBackend::advance_steward_watermark` (the same-act one and the idempotence one), never by
    /// this fixture.
    ///
    /// Settling the fingerprint matters even in tests that ignore the boundary: it is NULL on a fresh
    /// cogmap, NULL means "never snapshotted" means moved, so an unsettled map reports
    /// `exceeds_threshold` regardless of its counts.
    async fn settle_cursors(pool: &PgPool, cogmap: Uuid, watermark_event: Option<Uuid>) {
        // `steward_boundary_fingerprint($1)` is the FUNCTION applied to the bind parameter, not the
        // same-named column — a column reference cannot take an argument. Passing $1 rather than the
        // `id` column removes the question entirely, which is the form the production UPDATE uses too.
        sqlx::query(
            "UPDATE kb_cogmaps \
                SET steward_watermark_event_id = COALESCE($2, steward_watermark_event_id), \
                    steward_boundary_fingerprint = steward_boundary_fingerprint($1) \
              WHERE id = $1",
        )
        .bind(cogmap)
        .bind(watermark_event)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Settle **every** cogmap's boundary, not just the seeded one.
    ///
    /// Required by any test that asserts on the size of a `drift_sweep` result, and the reason is not
    /// the seeded map: a fresh migrated database already contains the **L0 kernel** cogmap
    /// (`system-default`, born by `20260625000001`), it is joined to the root team, and a plain new
    /// profile reaches it — verified by execution, two candidates come back from
    /// `steward_candidate_cogmaps` for a profile that owns one map. Its fingerprint is NULL, NULL means
    /// "never snapshotted" means moved, so it enters every sweep on the boundary arm and
    /// "exactly one drifted map" is false for a reason that has nothing to do with what the test is
    /// about.
    ///
    /// Settling puts these tests in the **steady state** they are written about. The one-time wave —
    /// every pre-existing map sweeping once after deploy, then settling — is real, intended, and
    /// covered by `a_never_snapshotted_boundary_reads_as_moved` rather than papered over here.
    async fn settle_all_boundaries(pool: &PgPool) {
        // Here the argument must be the `id` COLUMN, since this settles every row in one statement —
        // `steward_boundary_fingerprint(id)` is still the function applied to that column's value, as a
        // column reference cannot take an argument.
        sqlx::query(
            "UPDATE kb_cogmaps SET steward_boundary_fingerprint = steward_boundary_fingerprint(id)",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// The two stored cursors, read raw — `(watermark, boundary_fingerprint)`.
    async fn stored_cursors(pool: &PgPool, cogmap: Uuid) -> (Option<Uuid>, Option<String>) {
        sqlx::query_as(
            "SELECT steward_watermark_event_id, steward_boundary_fingerprint \
               FROM kb_cogmaps WHERE id = $1",
        )
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Share a context into a team's reach — one `kb_team_contexts` row, and (product decision 5)
    /// **no event whatsoever**. The write that moves the steward's boundary invisibly.
    async fn share_context_into_team(pool: &PgPool, context: Uuid, team: Uuid) {
        sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
            .bind(context)
            .bind(team)
            .execute(pool)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn delta_counts_resources_and_events_scoped_to_team_contexts(pool: PgPool) {
        let s = seed(&pool).await;
        // Settle the boundary (watermark untouched): this test is about the counted arm, and an
        // unsettled boundary would fire `exceeds_threshold` on its own.
        settle_cursors(&pool, s.cogmap, None).await;
        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        add_event(&pool, s.entity, "relationship_asserted", s.ctx).await;
        // The last event IN the team context — the delta's max_event_id, uuidv7-newest of the window.
        let last_team_event = add_event(&pool, s.entity, "block_mutated", s.ctx).await;
        // Noise in a context the team does not own — must be excluded from the delta AND from its
        // max_event_id, even though it is the newest event overall (inserted last → largest uuidv7).
        add_event(&pool, s.entity, "resource_created", s.other_ctx).await;

        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();

        assert_eq!(d.new_resources, 3, "3 resource_created in the team context");
        assert_eq!(
            d.new_events, 5,
            "all 5 team-context events, excluding the other context"
        );
        assert_eq!(
            d.max_event_id,
            Some(last_team_event),
            "max_event_id is the newest IN-WINDOW event, not the newer out-of-scope noise event"
        );
        assert_eq!(d.threshold, DEFAULT_STEWARD_INGEST_THRESHOLD);
        assert!(!d.boundary_moved, "boundary settled and untouched since");
        assert!(
            !d.exceeds_threshold,
            "3 < default threshold 5, and neither arm of the decision fires"
        );
        assert_eq!(d.watermark, None, "no watermark set yet");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn delta_max_event_id_is_none_when_window_empty(pool: PgPool) {
        let s = seed(&pool).await;
        // No events at all → an empty window → nothing to advance to.
        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(d.new_events, 0);
        assert_eq!(
            d.max_event_id, None,
            "empty window has no max_event_id — the tick skips the advance"
        );
    }

    /// The change-detection scope is the UNION of the joined teams' reachable contexts — not the
    /// intersection. A context reachable by ANY joined team counts, because this scope only drives the
    /// "did anything change?" trigger + watermark; cross-team leak-safety is enforced downstream at the
    /// resource grain (the cogmap-principal distillation), so over-approximating here is safe and a
    /// narrower intersection would under-trigger. The boundary is still real: a context NO joined team
    /// can reach is excluded.
    #[sqlx::test(migrations = "../../migrations")]
    async fn change_detection_scope_unions_joined_teams_reachable_contexts(pool: PgPool) {
        let s = seed(&pool).await;
        // Join a SECOND team to the cogmap. s.ctx is owned by the seed team only — team_b cannot reach
        // it — yet under the union it still counts (this is change detection, not a visibility gate).
        let team_b: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('team-b', 'Team B') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
            .bind(s.cogmap)
            .bind(team_b)
            .execute(&pool)
            .await
            .unwrap();

        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        // Noise in a context NO joined team can reach (outsider-owned) — the union's outer boundary.
        add_event(&pool, s.entity, "resource_created", s.other_ctx).await;

        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            d.new_resources, 3,
            "s.ctx is reachable by one joined team → in the union (change detection over-approximates; \
             leak-safety is downstream, not here)"
        );
        assert_eq!(
            d.new_events, 3,
            "s.other_ctx is reachable by no joined team → excluded even from the union"
        );
        assert!(
            d.max_event_id.is_some(),
            "the newest in-scope event is advanceable"
        );
    }

    /// The change-detection scope expands a joined team's reach UP the team DAG (shares inherit down):
    /// a context shared to a joined team's ANCESTOR is reachable by that team, so it counts. A context
    /// shared only to an unrelated team — one the cogmap is not joined to and that is not an ancestor
    /// of a joined team — does not.
    #[sqlx::test(migrations = "../../migrations")]
    async fn change_detection_scope_expands_shares_up_the_team_dag(pool: PgPool) {
        async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
            sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
                .bind(slug)
                .fetch_one(pool)
                .await
                .unwrap()
        }
        async fn share_ctx(pool: &PgPool, owner: Uuid, slug: &str, team: Uuid) -> Uuid {
            let ctx: Uuid = sqlx::query_scalar(
                "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
                 VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
            )
            .bind(owner)
            .bind(slug)
            .fetch_one(pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
                .bind(ctx)
                .bind(team)
                .execute(pool)
                .await
                .unwrap();
            ctx
        }

        // Parent P with child T (joined to the cogmap); an unrelated team U (not joined, not ancestor).
        let p = mk_team(&pool, "p").await;
        let t = mk_team(&pool, "t").await;
        let u = mk_team(&pool, "u").await;
        sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
            .bind(t)
            .bind(p)
            .execute(&pool)
            .await
            .unwrap();
        let member = insert_profile(&pool, "member").await;
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(t)
        .bind(member)
        .execute(&pool)
        .await
        .unwrap();
        let entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'e') RETURNING id",
        )
        .bind(member)
        .fetch_one(&pool)
        .await
        .unwrap();
        let telos: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let cogmap: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('map', $1) RETURNING id",
        )
        .bind(telos)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
            .bind(cogmap)
            .bind(t)
            .execute(&pool)
            .await
            .unwrap();

        let owner = insert_profile(&pool, "ctx_owner").await;
        // Shared to T's ANCESTOR P — T reaches it via up-traversal → in scope.
        let ctx_ancestor = share_ctx(&pool, owner, "cp", p).await;
        // Shared to an unrelated team the cogmap is not joined to → out of scope.
        let ctx_unrelated = share_ctx(&pool, owner, "cu", u).await;
        add_event(&pool, entity, "resource_created", ctx_ancestor).await;
        add_event(&pool, entity, "resource_created", ctx_unrelated).await;

        let d = ingest_delta(&pool, member.into(), cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            d.new_resources, 1,
            "the ancestor-shared context is reachable by the joined team via up-traversal; the \
             unrelated team's context is not reachable by any joined team"
        );
    }

    /// The counted arm, unchanged: with the boundary settled (so only counts can decide),
    /// `exceeds_threshold` is still exactly `new_resources >= threshold`, `>=` boundary included.
    #[sqlx::test(migrations = "../../migrations")]
    async fn threshold_gates_on_new_resources(pool: PgPool) {
        let s = seed(&pool).await;
        settle_cursors(&pool, s.cogmap, None).await;
        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }

        let below = ingest_delta(&pool, s.member.into(), s.cogmap.into(), Some(5))
            .await
            .unwrap();
        assert!(!below.boundary_moved, "the boundary arm is quiet here");
        assert!(!below.exceeds_threshold, "3 < 5");

        let at_boundary = ingest_delta(&pool, s.member.into(), s.cogmap.into(), Some(3))
            .await
            .unwrap();
        assert!(at_boundary.exceeds_threshold, "3 >= 3 (>= boundary)");
        assert_eq!(at_boundary.threshold, 3);
    }

    /// A settled boundary is what makes an unsettled one legible: on a cogmap that has never been
    /// snapshotted the fingerprint is NULL, and NULL is "unknown", not "unchanged" — so the steward
    /// runs once even with nothing counted. Over-trigger, deliberately.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_never_snapshotted_boundary_reads_as_moved(pool: PgPool) {
        let s = seed(&pool).await;

        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(d.new_resources, 0, "nothing counted at all");
        assert!(
            d.boundary_moved,
            "NULL stored fingerprint → unknown → moved"
        );
        assert!(
            d.exceeds_threshold,
            "the boundary arm alone decides the steward should run"
        );
        assert!(
            d.boundary_fingerprint.is_some(),
            "the live fingerprint is computed regardless, and is what a run stores back"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn advancing_watermark_shrinks_the_delta(pool: PgPool) {
        let s = seed(&pool).await;
        add_event(&pool, s.entity, "resource_created", s.ctx).await;
        let e2 = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        add_event(&pool, s.entity, "resource_created", s.ctx).await;
        let e4 = add_event(&pool, s.entity, "relationship_asserted", s.ctx).await;

        // A team member is not a cogmap author by default (D3b) — grant write, then advance to e2.
        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());
        let ack = backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: Some(e2),
                boundary_fingerprint: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();
        assert_eq!(
            ack.value.watermark,
            Some(e2),
            "the ack reports what was stored"
        );

        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(d.watermark, Some(e2));
        assert_eq!(d.new_resources, 1, "only the resource_created after e2");
        assert_eq!(
            d.new_events, 2,
            "the trailing resource_created + relationship"
        );
        assert_eq!(
            d.max_event_id,
            Some(e4),
            "max_event_id is the newest event after the watermark, ready for the next advance"
        );
    }

    /// The window gate is unchanged by `event_id` having become optional: absence skips the check
    /// (there is no target), but a SUPPLIED id faces exactly the check it always did. This is the test
    /// that would catch "optional" being mistaken for "unvalidated".
    #[sqlx::test(migrations = "../../migrations")]
    async fn advance_rejects_event_outside_ingest_window(pool: PgPool) {
        let s = seed(&pool).await;
        // An event the cogmap ingests (team context) and one it does not (a context the team
        // neither owns nor was shared) — same emitter, so the only difference is the anchor.
        let in_window = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        let out_of_window = add_event(&pool, s.entity, "resource_created", s.other_ctx).await;

        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());

        // Advancing to an event outside the cogmap's ingest window is rejected — the watermark can
        // never move past content the steward did not (and could not) process.
        let err = backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: Some(out_of_window),
                boundary_fingerprint: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, TemperError::NotFound(_)),
            "out-of-window event → 404, no advance"
        );

        // The in-window event (what a real delta's max_event_id always is) advances cleanly.
        let ack = backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: Some(in_window),
                boundary_fingerprint: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();
        assert_eq!(ack.value.watermark, Some(in_window));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_returns_only_drifted_maps_most_drifted_first(pool: PgPool) {
        let s = seed(&pool).await;
        // The sweep gates on authorship since `20260805000010`; team membership alone is not enough.
        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        // Settle EVERY boundary, not just this map's — the L0 kernel is a candidate too and would
        // otherwise sweep on the boundary arm. See `settle_all_boundaries`.
        settle_all_boundaries(&pool).await;
        // 6 resource_created in the team context → above default threshold 5.
        for _ in 0..6 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        let rows = drift_sweep(&pool, s.member.into(), None).await.unwrap();
        assert_eq!(rows.len(), 1, "the one drifted, readable, team-joined map");
        assert_eq!(rows[0].cogmap_id, s.cogmap);
        assert_eq!(rows[0].new_resources, 6);
        assert!(!rows[0].boundary_moved, "swept on the counted arm");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_excludes_below_threshold_and_unreadable(pool: PgPool) {
        let s = seed(&pool).await;
        // Authorship is the sweep's candidacy gate since `20260805000010`; this test is about the
        // threshold and the read gate, so give the member the grant and vary only those.
        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        // Settle EVERY boundary: this test is about the counted arm and the read gate, and any
        // unsettled map — this one or the L0 kernel — would enter the sweep on the other arm.
        settle_all_boundaries(&pool).await;
        for _ in 0..2 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        // Below threshold for the member.
        assert!(drift_sweep(&pool, s.member.into(), None)
            .await
            .unwrap()
            .is_empty());
        // Push above threshold: the outsider still cannot read the map → never a candidate.
        for _ in 0..6 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        assert!(drift_sweep(&pool, s.outsider.into(), None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            drift_sweep(&pool, s.member.into(), None)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    /// A map the principal can READ but not AUTHOR is never swept — the regression test for the
    /// production defect `20260805000010` closes.
    ///
    /// The sweep queues work that terminates in `advance_steward_watermark`, whose gate is an
    /// explicit write grant. While candidacy was readability-scoped, a readable-but-unauthorable map
    /// was dispatched every run and refused every run. The L0 kernel is exactly that map for every
    /// principal — joined to the root `temper-system` team so all may read it, authorable by nobody
    /// by design — which is why its `steward_watermark_event_id` was still NULL after 15 refusals a
    /// day, and why it had never once materialized.
    ///
    /// Drifted hard enough to sweep on the counted arm, so the ONLY thing keeping it out is the
    /// authorship conjunct.
    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_skips_a_readable_but_unauthorable_map(pool: PgPool) {
        let s = seed(&pool).await;
        settle_all_boundaries(&pool).await;
        for _ in 0..6 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }

        // The member reads the map by team membership and holds no write grant — precisely the L0
        // shape. Assert the read half explicitly, or an excluded map proves nothing: it would be
        // indistinguishable from a map that was never a candidate.
        let readable: bool =
            sqlx::query_scalar("SELECT anchor_readable_by_profile($1, 'kb_cogmaps', $2)")
                .bind(s.member)
                .bind(s.cogmap)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(readable, "precondition: the member READS the map");

        assert!(
            drift_sweep(&pool, s.member.into(), None)
                .await
                .unwrap()
                .is_empty(),
            "a readable but unauthorable map must not be dispatched work it cannot perform"
        );

        // Grant authorship and nothing else changes — so authorship, not drift or readability, is
        // what was withholding it.
        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        assert_eq!(
            drift_sweep(&pool, s.member.into(), None)
                .await
                .unwrap()
                .len(),
            1,
            "the same drifted map sweeps once it is authorable"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn unreadable_cogmap_is_not_found(pool: PgPool) {
        let s = seed(&pool).await;
        let err = ingest_delta(&pool, s.outsider.into(), s.cogmap.into(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ApiError::NotFound(_)),
            "deny → 404, no existence oracle"
        );
    }

    /// **This is the production refusal.** The steward met exactly this — read on L0 via the root
    /// team, no write grant on it, once per run for eight days — and because the `403` named nothing,
    /// it could not form a corrective hypothesis and probed until it found a real bypass. So the
    /// member half below asserts the refusal *says which grant is missing*, not merely that it
    /// refused.
    ///
    /// The outsider half is what makes that safe, and neither half means anything alone: the gate's
    /// `WHERE` carries `anchor_readable_by_profile`, so a caller who cannot read the map never
    /// reaches the `403` at all — they get the existence-hiding `404`. Every caller who *does* reach
    /// it has already proven they read the map, which is precisely the standing that makes naming
    /// the capability a disclosure of nothing new.
    #[sqlx::test(migrations = "../../migrations")]
    async fn advance_requires_cogmap_write_grant(pool: PgPool) {
        let s = seed(&pool).await;
        let e = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        let cmd = || AdvanceStewardWatermark {
            cogmap: s.cogmap.into(),
            event_id: Some(e),
            boundary_fingerprint: None,
            origin: Surface::ApiHttp,
        };

        // Member can READ the cogmap (team join) but has no WRITE grant → 403, and the 403 talks.
        let member_backend = DbBackend::new(pool.clone(), s.member.into());
        let err = member_backend
            .advance_steward_watermark(cmd())
            .await
            .unwrap_err();
        let TemperError::ForbiddenDetail(msg) = &err else {
            panic!("read but not write → 403, in the disclosing dialect: {err:?}");
        };
        assert!(
            msg.contains("write grant") && msg.contains(&s.cogmap.to_string()),
            "the refusal must name the missing capability AND the map it was refused on: {msg:?}"
        );

        // Outsider cannot even read the cogmap → NotFound (404), no existence oracle.
        let outsider_backend = DbBackend::new(pool.clone(), s.outsider.into());
        let err2 = outsider_backend
            .advance_steward_watermark(cmd())
            .await
            .unwrap_err();
        assert!(matches!(err2, TemperError::NotFound(_)), "unreadable → 404");
    }

    // ── The boundary arm ───────────────────────────────────────────────────────────────────────
    //
    // The counts are taken INSIDE `steward_team_contexts` (team-OWNED ∪ SHARED). That set can move
    // without emitting a countable event, by either of its two halves — a share, or an ownership
    // transfer — so each half gets its own witness below. Both are built so the counts are pinned at
    // ZERO across the move: the material that enters scope predates the watermark, which is the case
    // a count-only gate can never recover from (it is not merely late; it never fires).

    /// SHARED half: a `kb_team_contexts` row brings a whole context into the steward's scope, emits
    /// no event, and — because that context's material predates the watermark — moves no count.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_shared_context_moves_the_boundary_with_nothing_counted(pool: PgPool) {
        let s = seed(&pool).await;
        // Three resources that already exist in a context no joined team can reach.
        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.other_ctx).await;
        }
        // One in-scope event, then the state a completed run leaves: watermark here, boundary snapshotted.
        let wm = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        settle_cursors(&pool, s.cogmap, Some(wm)).await;

        let quiet = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            quiet.new_events, 0,
            "caught up: nothing after the watermark"
        );
        assert!(!quiet.boundary_moved, "boundary just snapshotted");
        assert!(
            !quiet.exceeds_threshold,
            "baseline: the steward should not run"
        );

        // THE DEFECT THIS CLOSES: one INSERT, zero events, and three resources enter the scope the
        // steward distills from.
        share_context_into_team(&pool, s.other_ctx, s.team).await;

        let after = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            after.new_resources, 0,
            "the share emits nothing, and the material it admitted predates the watermark — the \
             counted arm is blind to this forever, not merely late"
        );
        assert_eq!(
            after.new_events, 0,
            "no event of any type accompanies a share"
        );
        assert!(after.boundary_moved, "the scope itself changed shape");
        assert!(
            after.exceeds_threshold,
            "the boundary arm alone must fire — with a count-only gate this map never ticks again"
        );
    }

    /// OWNS half: the same invisibility through the other door — transferring a context's ownership
    /// onto the joined team. `kb_contexts` is not the event spine, so the scope grows uncounted.
    #[sqlx::test(migrations = "../../migrations")]
    async fn reassigning_a_context_onto_the_joined_team_moves_the_boundary(pool: PgPool) {
        let s = seed(&pool).await;
        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.other_ctx).await;
        }
        let wm = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        settle_cursors(&pool, s.cogmap, Some(wm)).await;

        // A `context_reassigned`-style move: owner becomes the joined team, so the context enters the
        // boundary's OWNED half. (The row move is what the boundary observes; no `resource_created`
        // accompanies material that already existed.)
        sqlx::query("UPDATE kb_contexts SET owner_table = 'kb_teams', owner_id = $2 WHERE id = $1")
            .bind(s.other_ctx)
            .bind(s.team)
            .execute(&pool)
            .await
            .unwrap();

        let after = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            after.new_resources, 0,
            "the transferred context's resources predate the watermark"
        );
        assert!(after.boundary_moved, "the OWNED half of the scope grew");
        assert!(after.exceeds_threshold, "the boundary arm fires");
    }

    /// Idempotence: once a run stores the fingerprint it observed, the same boundary reads as
    /// unchanged. Without this the boundary arm would latch on and the steward would tick forever.
    #[sqlx::test(migrations = "../../migrations")]
    async fn advancing_with_the_delta_fingerprint_settles_the_boundary(pool: PgPool) {
        let s = seed(&pool).await;
        add_event(&pool, s.entity, "resource_created", s.ctx).await;

        let first = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert!(first.boundary_moved, "fresh cogmap: never snapshotted");

        assert!(
            first.max_event_id.is_some(),
            "this run has an event to advance to"
        );

        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());
        backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: first.max_event_id,
                boundary_fingerprint: first.boundary_fingerprint.clone(),
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();

        let second = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            second.boundary_fingerprint, first.boundary_fingerprint,
            "an unchanged boundary computes to the same fingerprint"
        );
        assert!(!second.boundary_moved, "stored == live → not moved");
        assert!(
            !second.exceeds_threshold,
            "nothing counted and nothing moved → the steward stays put"
        );
    }

    /// One act, both cursors. How far the run read and what shape its scope had are stored together,
    /// so they cannot diverge — a change that advances only one of them fails right here.
    #[sqlx::test(migrations = "../../migrations")]
    async fn advance_moves_the_watermark_and_the_boundary_fingerprint_in_one_act(pool: PgPool) {
        let s = seed(&pool).await;
        let e = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        let fingerprint = d
            .boundary_fingerprint
            .expect("the read computes a live fingerprint");

        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());
        let ack = backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: Some(e),
                boundary_fingerprint: Some(fingerprint.clone()),
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();

        let (watermark, stored) = stored_cursors(&pool, s.cogmap).await;
        assert_eq!(watermark, Some(e), "the event watermark moved");
        assert_eq!(
            stored,
            Some(fingerprint.clone()),
            "and so did the boundary fingerprint, in the same UPDATE"
        );
        // The ack is read back from that same UPDATE, so it must agree with the table. Asserting it
        // against the COLUMNS rather than against the arguments is the point: an ack that echoed the
        // request would pass this even if the UPDATE wrote nothing.
        assert_eq!(
            ack.value.watermark, watermark,
            "the ack reports stored state"
        );
        assert_eq!(ack.value.boundary_fingerprint, stored);
        assert_eq!(ack.value.cogmap_id, s.cogmap);
    }

    // ── D5/D6: the cursors are independently optional ──────────────────────────────────────────

    /// D5. A boundary-only advance: no event id, so the watermark must not move — while the boundary
    /// still gets recorded. Asserting the watermark COLUMN is unchanged (not merely that the call
    /// returned Ok) is what makes this a test of the COALESCE rather than of the happy path.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_boundary_only_advance_stores_the_fingerprint_and_leaves_the_watermark(pool: PgPool) {
        let s = seed(&pool).await;
        let e = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        settle_cursors(&pool, s.cogmap, Some(e)).await;
        // Move the boundary so there is something new to record, with no event to advance to.
        share_context_into_team(&pool, s.other_ctx, s.team).await;
        let (watermark_before, fingerprint_before) = stored_cursors(&pool, s.cogmap).await;

        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(d.max_event_id, None, "empty window: nothing to advance to");

        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());
        let ack = backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: None,
                boundary_fingerprint: d.boundary_fingerprint.clone(),
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();

        let (watermark_after, fingerprint_after) = stored_cursors(&pool, s.cogmap).await;
        assert_eq!(
            watermark_after, watermark_before,
            "no event id supplied → the watermark column is untouched, not nulled"
        );
        assert_eq!(
            watermark_after,
            Some(e),
            "and it still holds the event it held"
        );
        assert_ne!(
            fingerprint_after, fingerprint_before,
            "the boundary moved, so its recorded digest must have changed"
        );
        assert_eq!(fingerprint_after, d.boundary_fingerprint);
        assert_eq!(ack.value.watermark, watermark_after);
        assert_eq!(ack.value.boundary_fingerprint, fingerprint_after);
    }

    /// **The hot loop, asserted shut.** This is the case the whole boundary mechanism exists for and
    /// the one it could not previously survive: a context is shared in, so the boundary moves with
    /// zero countable events, which means the delta has no `max_event_id` — and when an event id was
    /// mandatory, such a run could record nothing and re-fired on every tick, forever. If this test
    /// ever fails on the final assertion, the steward is in a distillation loop in production.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_boundary_move_with_no_event_to_advance_to_does_not_re_fire_forever(pool: PgPool) {
        let s = seed(&pool).await;
        // Material that predates the watermark, in a context no joined team can reach.
        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.other_ctx).await;
        }
        let wm = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        settle_cursors(&pool, s.cogmap, Some(wm)).await;

        share_context_into_team(&pool, s.other_ctx, s.team).await;

        let fired = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert!(fired.exceeds_threshold, "the boundary arm says run");
        assert_eq!(
            fired.max_event_id, None,
            "and there is NO event to advance to — the two facts that used to be irreconcilable"
        );

        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());
        backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: fired.max_event_id,
                boundary_fingerprint: fired.boundary_fingerprint.clone(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("a run with no event id must still be able to record its boundary");

        let next = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert!(
            !next.boundary_moved,
            "the boundary is settled — this assertion IS the loop being shut"
        );
        assert!(!next.exceeds_threshold, "and the next tick stands down");
    }

    /// D6. A caller that supplies no fingerprint still settles the boundary — the server computes one
    /// at write time. Storing NULL instead would mean "never snapshotted", re-firing this cogmap on
    /// every tick forever: the same hot loop through a different door. The cost is that a boundary
    /// change during the run is absorbed unseen, which is why callers holding the delta's value pass it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_advance_without_a_fingerprint_settles_the_boundary_anyway(pool: PgPool) {
        let s = seed(&pool).await;
        let e = add_event(&pool, s.entity, "resource_created", s.ctx).await;

        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());
        let ack = backend
            .advance_steward_watermark(AdvanceStewardWatermark {
                cogmap: s.cogmap.into(),
                event_id: Some(e),
                boundary_fingerprint: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();

        let (_, stored) = stored_cursors(&pool, s.cogmap).await;
        assert!(
            stored.is_some(),
            "the fallback computed a digest — NULL here would be the never-snapshotted hot loop"
        );
        assert_eq!(
            ack.value.boundary_fingerprint, stored,
            "and the ack reports the fallback's value, so the caller can see it was filled in"
        );

        let next = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert!(
            !next.boundary_moved,
            "settled, despite the caller sending nothing"
        );
        assert!(!next.exceeds_threshold);
    }

    /// The cron path carries the boundary arm too. A fix that landed only in `ingest_delta` would
    /// pass every other test here while `steward_dispatch_tick` — which enqueues from THIS sweep —
    /// stayed blind to the exact case the fix exists for.
    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_includes_a_map_whose_boundary_moved_below_threshold(pool: PgPool) {
        let s = seed(&pool).await;
        // The sweep gates on authorship since `20260805000010`; this test varies the boundary arm.
        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        // Every boundary, so the empty baseline below is a real baseline — see `settle_all_boundaries`.
        settle_all_boundaries(&pool).await;
        for _ in 0..2 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        assert!(
            drift_sweep(&pool, s.member.into(), None)
                .await
                .unwrap()
                .is_empty(),
            "2 < default threshold 5, boundary settled → neither arm"
        );

        share_context_into_team(&pool, s.other_ctx, s.team).await;

        let rows = drift_sweep(&pool, s.member.into(), None).await.unwrap();
        assert_eq!(rows.len(), 1, "the sweep must surface the moved map");
        assert_eq!(rows[0].cogmap_id, s.cogmap);
        assert_eq!(
            rows[0].new_resources, 2,
            "still below the threshold — this row is in the sweep on the boundary arm, not the count"
        );
        assert!(rows[0].boundary_moved, "and it says so");
    }
}
