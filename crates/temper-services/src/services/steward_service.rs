//! Read path for the team-self-cognition steward's ingest trigger (T4a).
//!
//! Service-direct (the read-path convention): the surface passes a resolved cogmap id + optional
//! threshold; this gates on `anchor_readable_by_profile` and returns the [`IngestDelta`]. The write
//! side (advancing the watermark) routes through the `Backend` trait / `DbBackend`, not here.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::steward::{IngestDelta, DEFAULT_STEWARD_INGEST_THRESHOLD};

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
               new_events    AS "new_events!: i64"
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
        threshold,
        exceeds_threshold: row.new_resources >= threshold,
    })
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
        add_event(&pool, s.entity, "block_mutated", s.ctx).await;
        // Noise in a context the team does not own — must be excluded from the delta.
        add_event(&pool, s.entity, "resource_created", s.other_ctx).await;

        let d = ingest_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();

        assert_eq!(d.new_resources, 3, "3 resource_created in the team context");
        assert_eq!(
            d.new_events, 5,
            "all 5 team-context events, excluding the other context"
        );
        assert_eq!(d.threshold, DEFAULT_STEWARD_INGEST_THRESHOLD);
        assert!(!d.exceeds_threshold, "3 < default threshold 5");
        assert_eq!(d.watermark, None, "no watermark set yet");
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
        add_event(&pool, s.entity, "relationship_asserted", s.ctx).await;

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
