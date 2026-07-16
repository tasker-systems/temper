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
    // One query does the read-gate AND the watermark lookup: an absent row means the cogmap does not
    // exist OR the caller cannot read it — both surface as NotFound. The column is nullable (NULL =
    // never run), so the scalar is `Option<Uuid>` inside the fetch_optional `Option`.
    let watermark: Option<Uuid> = sqlx::query_scalar!(
        r#"
        SELECT steward_watermark_event_id AS "watermark: Uuid"
          FROM kb_cogmaps
         WHERE id = $1
           AND anchor_readable_by_profile($2, 'kb_cogmaps', $1)
        "#,
        *cogmap_id,
        *principal,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    let row = sqlx::query!(
        r#"
        SELECT new_resources AS "new_resources!: i64",
               new_events    AS "new_events!: i64",
               max_event_id  AS "max_event_id: Uuid"
          FROM steward_ingest_delta($1, $2)
        "#,
        *cogmap_id,
        watermark,
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
        exceeds_threshold: row.new_resources >= threshold,
    })
}

/// Sweep all team-joined cogmaps the principal can read, returning those whose ingest delta clears
/// `threshold`, most-drifted-first. The privileged case (the steward app-principal) simply has broad
/// read; the gate is the same `anchor_readable_by_profile` every read uses — not a bypass.
pub async fn drift_sweep(
    pool: &PgPool,
    principal: ProfileId,
    threshold: Option<i64>,
) -> ApiResult<Vec<DriftSweepRow>> {
    let threshold = threshold.unwrap_or(DEFAULT_STEWARD_INGEST_THRESHOLD);
    let rows = sqlx::query!(
        r#"
        SELECT cogmap_id     AS "cogmap_id!: Uuid",
               watermark     AS "watermark: Uuid",
               new_resources AS "new_resources!: i64",
               new_events    AS "new_events!: i64"
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
        })
        .collect())
}

/// All team-joined cogmaps the principal can read (the materialize fan-out candidate set).
pub async fn candidate_cogmaps(pool: &PgPool, principal: ProfileId) -> ApiResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar!(
        r#"SELECT cogmap_id AS "id!: Uuid" FROM steward_candidate_cogmaps($1)"#,
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn delta_counts_resources_and_events_scoped_to_team_contexts(pool: PgPool) {
        let s = seed(&pool).await;
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
        assert!(!d.exceeds_threshold, "3 < default threshold 5");
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

    /// The ingest scope is the producer-INTERSECTION of the cogmap's joined teams, not the union. A
    /// cogmap joined to two teams must ingest only contexts BOTH can reach — otherwise a high-privilege
    /// team's context leaks down into the shared map that the other team reads (issue #459 review).
    #[sqlx::test(migrations = "../../migrations")]
    async fn ingest_scope_is_producer_intersection_across_joined_teams(pool: PgPool) {
        let s = seed(&pool).await;
        // s.ctx is OWNED by the seed team (already joined to s.cogmap). Join a SECOND team to the same
        // cogmap: the map now spans two teams, so its scope collapses to what both reach.
        // The seed team by slug — NOT via `kb_team_members WHERE profile_id = member`, which is
        // ambiguous: the auto-join trigger also enrolls the member in the `temper-system` root team,
        // so that query returns two rows and picks a team the cogmap is not joined to.
        let seed_team: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'team'")
            .fetch_one(&pool)
            .await
            .unwrap();
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

        // Activity in s.ctx — reachable by the seed team only. team_b cannot reach it, so with the
        // cogmap now spanning both teams it drops out of the intersection: it must NOT count. (Under
        // the old union it would have — the leak.)
        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        let leaked = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            leaked.new_resources, 0,
            "s.ctx is reachable by only one of the two joined teams — excluded by the intersection"
        );
        assert_eq!(leaked.new_events, 0);
        assert_eq!(leaked.max_event_id, None);

        // A context SHARED to BOTH teams IS in the intersection → its activity counts.
        let shared_ctx: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'shared', 'Shared') RETURNING id",
        )
        .bind(s.outsider)
        .fetch_one(&pool)
        .await
        .unwrap();
        for team in [seed_team, team_b] {
            sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
                .bind(shared_ctx)
                .bind(team)
                .execute(&pool)
                .await
                .unwrap();
        }
        add_event(&pool, s.entity, "resource_created", shared_ctx).await;

        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();
        assert_eq!(
            d.new_resources, 1,
            "the shared context is in both teams' reach → in the intersection → counted"
        );
        assert!(
            d.max_event_id.is_some(),
            "the shared-context event is the window's newest, so it is advanceable"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn threshold_gates_on_new_resources(pool: PgPool) {
        let s = seed(&pool).await;
        for _ in 0..3 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }

        let below = ingest_delta(&pool, s.member.into(), s.cogmap.into(), Some(5))
            .await
            .unwrap();
        assert!(!below.exceeds_threshold, "3 < 5");

        let at_boundary = ingest_delta(&pool, s.member.into(), s.cogmap.into(), Some(3))
            .await
            .unwrap();
        assert!(at_boundary.exceeds_threshold, "3 >= 3 (>= boundary)");
        assert_eq!(at_boundary.threshold, 3);
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
                event_id: e2,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();
        assert_eq!(ack.value, e2);

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
                event_id: out_of_window,
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
                event_id: in_window,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();
        assert_eq!(ack.value, in_window);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_returns_only_drifted_maps_most_drifted_first(pool: PgPool) {
        let s = seed(&pool).await;
        // 6 resource_created in the team context → above default threshold 5.
        for _ in 0..6 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        let rows = drift_sweep(&pool, s.member.into(), None).await.unwrap();
        assert_eq!(rows.len(), 1, "the one drifted, readable, team-joined map");
        assert_eq!(rows[0].cogmap_id, s.cogmap);
        assert_eq!(rows[0].new_resources, 6);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_excludes_below_threshold_and_unreadable(pool: PgPool) {
        let s = seed(&pool).await;
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn unreadable_cogmap_is_not_found(pool: PgPool) {
        let s = seed(&pool).await;
        let err = ingest_delta(&pool, s.outsider.into(), s.cogmap.into(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ApiError::NotFound),
            "deny → 404, no existence oracle"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn advance_requires_cogmap_write_grant(pool: PgPool) {
        let s = seed(&pool).await;
        let e = add_event(&pool, s.entity, "resource_created", s.ctx).await;
        let cmd = || AdvanceStewardWatermark {
            cogmap: s.cogmap.into(),
            event_id: e,
            origin: Surface::ApiHttp,
        };

        // Member can READ the cogmap (team join) but has no WRITE grant → Forbidden (403).
        let member_backend = DbBackend::new(pool.clone(), s.member.into());
        let err = member_backend
            .advance_steward_watermark(cmd())
            .await
            .unwrap_err();
        assert!(
            matches!(err, TemperError::Forbidden),
            "read but not write → 403"
        );

        // Outsider cannot even read the cogmap → NotFound (404), no existence oracle.
        let outsider_backend = DbBackend::new(pool.clone(), s.outsider.into());
        let err2 = outsider_backend
            .advance_steward_watermark(cmd())
            .await
            .unwrap_err();
        assert!(matches!(err2, TemperError::NotFound(_)), "unreadable → 404");
    }
}
