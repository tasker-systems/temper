//! Read path for cron-driven region materialization on a drift threshold (T4b).
//!
//! Service-direct (the read-path convention): the surface passes a resolved cogmap id + optional
//! threshold; this gates on `anchor_readable_by_profile` and returns the [`MaterializeDelta`]. The
//! write side (running the materialize when over threshold) routes through the `Backend` trait /
//! `DbBackend`, not here.
//!
//! The delta reuses the SAME drift signal as the "is a recorded fingerprint stale?" gate — a cheap
//! `count(*)` of formation events since `shape_materialized_event_id` — deliberately cheaper than the
//! materialize it guards.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::materialize::{MaterializeDelta, DEFAULT_MATERIALIZE_THRESHOLD};

/// Compute the materialize delta for a cognitive map: how many formation events have landed on the
/// cogmap since it was last materialized, and whether that clears `threshold`.
///
/// Auth: the caller must be able to READ the cogmap (`anchor_readable_by_profile`). A cogmap the
/// caller cannot see is reported as `NotFound` (show-deny → 404, never leaking existence).
pub async fn materialize_delta(
    pool: &PgPool,
    principal: ProfileId,
    cogmap_id: CogmapId,
    threshold: Option<i64>,
) -> ApiResult<MaterializeDelta> {
    // One query does the read-gate AND the materialize-watermark lookup: an absent row means the
    // cogmap does not exist OR the caller cannot read it — both surface as NotFound. The column is
    // nullable (NULL = never materialized), so the scalar is `Option<Uuid>` inside the fetch_optional
    // `Option`.
    let watermark: Option<Uuid> = sqlx::query_scalar!(
        r#"
        SELECT shape_materialized_event_id AS "watermark: Uuid"
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

    let formation_events = temper_substrate::replay::formation_touched_count_since(
        pool,
        HomeAnchor::Cogmap(cogmap_id),
        watermark,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let threshold = threshold.unwrap_or(DEFAULT_MATERIALIZE_THRESHOLD);
    Ok(MaterializeDelta {
        cogmap_id: *cogmap_id,
        watermark,
        formation_events,
        threshold,
        exceeds_threshold: formation_events >= threshold,
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use crate::backend::DbBackend;
    use sqlx::PgPool;
    use temper_core::error::TemperError;
    use temper_workflow::operations::{Backend, MaterializeOnThreshold, Surface};

    /// A minimal cogmap graph: a member profile (in the team joined to the cogmap), an outsider (no
    /// access), the cogmap, and an emitter entity for synthesizing map-anchored events.
    struct Seeded {
        member: Uuid,
        outsider: Uuid,
        cogmap: Uuid,
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
            entity,
        }
    }

    /// Append a synthetic event of `type_name` anchored to the COGMAP (the in-cogmap formation
    /// scope), returning its id.
    async fn add_cogmap_event(pool: &PgPool, entity: Uuid, type_name: &str, cogmap: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = $1), $2, 'kb_cogmaps', $3) RETURNING id",
        )
        .bind(type_name)
        .bind(entity)
        .bind(cogmap)
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
    async fn delta_counts_formation_events_scoped_to_the_cogmap(pool: PgPool) {
        let s = seed(&pool).await;
        // Formation events on the cogmap: 2 structural + 1 content = 3 counted.
        add_cogmap_event(&pool, s.entity, "resource_created", s.cogmap).await;
        add_cogmap_event(&pool, s.entity, "relationship_asserted", s.cogmap).await;
        add_cogmap_event(&pool, s.entity, "block_mutated", s.cogmap).await;
        // A non-formation event on the cogmap — excluded from the count.
        add_cogmap_event(&pool, s.entity, "region_materialized", s.cogmap).await;

        let d = materialize_delta(&pool, s.member.into(), s.cogmap.into(), None)
            .await
            .unwrap();

        assert_eq!(
            d.formation_events, 3,
            "2 structural + 1 content formation events"
        );
        assert_eq!(d.threshold, DEFAULT_MATERIALIZE_THRESHOLD);
        assert!(!d.exceeds_threshold, "3 < default threshold 5");
        assert_eq!(d.watermark, None, "never materialized yet");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn threshold_gates_on_formation_events(pool: PgPool) {
        let s = seed(&pool).await;
        for _ in 0..3 {
            add_cogmap_event(&pool, s.entity, "resource_created", s.cogmap).await;
        }

        let below = materialize_delta(&pool, s.member.into(), s.cogmap.into(), Some(5))
            .await
            .unwrap();
        assert!(!below.exceeds_threshold, "3 < 5");

        let at_boundary = materialize_delta(&pool, s.member.into(), s.cogmap.into(), Some(3))
            .await
            .unwrap();
        assert!(at_boundary.exceeds_threshold, "3 >= 3 (>= boundary)");
        assert_eq!(at_boundary.threshold, 3);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn unreadable_cogmap_is_not_found(pool: PgPool) {
        let s = seed(&pool).await;
        let err = materialize_delta(&pool, s.outsider.into(), s.cogmap.into(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ApiError::NotFound),
            "deny → 404, no existence oracle"
        );
    }

    /// Below threshold, the trigger is an idempotent no-op: it returns `materialized: false` with no
    /// regions and never touches the substrate (so this needs no embeddings / lens). Proves the cheap
    /// gate short-circuits before the load-and-cluster path.
    #[sqlx::test(migrations = "../../migrations")]
    async fn trigger_below_threshold_is_a_noop(pool: PgPool) {
        let s = seed(&pool).await;
        add_cogmap_event(&pool, s.entity, "resource_created", s.cogmap).await;
        add_cogmap_event(&pool, s.entity, "relationship_asserted", s.cogmap).await;

        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());
        let ack = backend
            .materialize_on_threshold(MaterializeOnThreshold {
                cogmap: s.cogmap.into(),
                threshold: Some(5),
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();

        assert!(!ack.value.materialized, "2 < 5 → no-op");
        assert_eq!(ack.value.formation_events, 2);
        assert_eq!(ack.value.threshold, 5);
        assert_eq!(ack.value.regions, None);
        assert_eq!(ack.value.membership_fingerprint, None);
    }

    /// Over threshold, the trigger runs the REAL incremental-materialize path end to end: an empty
    /// cogmap materializes to 0 regions (no members ⇒ no embeddings needed), fires `region_materialized`,
    /// and advances `shape_materialized_event_id`. A second call then sees a below-threshold delta (the
    /// synthetic formation events predate the new watermark) and no-ops — proving the over-threshold
    /// branch, the watermark advance via the materialize projection, and idempotency, without seeding an
    /// embedded corpus. `telos-default` resolves to the global (cogmap_id IS NULL) seeded lens.
    #[sqlx::test(migrations = "../../migrations")]
    async fn trigger_over_threshold_materializes_and_advances_watermark(pool: PgPool) {
        let s = seed(&pool).await;
        for _ in 0..3 {
            add_cogmap_event(&pool, s.entity, "resource_created", s.cogmap).await;
        }
        grant_cogmap_write(&pool, s.cogmap, s.member).await;
        let backend = DbBackend::new(pool.clone(), s.member.into());

        let first = backend
            .materialize_on_threshold(MaterializeOnThreshold {
                cogmap: s.cogmap.into(),
                threshold: Some(3),
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();
        assert!(first.value.materialized, "3 >= 3 → materialize runs");
        assert_eq!(first.value.formation_events, 3);
        assert_eq!(first.value.regions, Some(0), "empty cogmap → 0 regions");
        assert!(
            first.value.membership_fingerprint.is_some(),
            "a materialize that ran carries its fingerprint"
        );

        // The materialize advanced shape_materialized_event_id past the synthetic events → the delta is
        // now below threshold → idempotent no-op. (Robust: assert the no-op, not an exact recount.)
        let second = backend
            .materialize_on_threshold(MaterializeOnThreshold {
                cogmap: s.cogmap.into(),
                threshold: Some(3),
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();
        assert!(
            !second.value.materialized,
            "watermark advanced → below threshold → idempotent no-op"
        );
    }

    /// The trigger is auth-before-write: a member who can READ the cogmap but has no WRITE grant is
    /// Forbidden; an outsider who cannot read it is NotFound (no existence oracle) — both checked
    /// before the threshold gate, so neither touches the substrate.
    #[sqlx::test(migrations = "../../migrations")]
    async fn trigger_requires_cogmap_write_grant(pool: PgPool) {
        let s = seed(&pool).await;
        let cmd = || MaterializeOnThreshold {
            cogmap: s.cogmap.into(),
            threshold: None,
            origin: Surface::ApiHttp,
        };

        let member_backend = DbBackend::new(pool.clone(), s.member.into());
        let err = member_backend
            .materialize_on_threshold(cmd())
            .await
            .unwrap_err();
        assert!(
            matches!(err, TemperError::Forbidden),
            "read but not write → 403"
        );

        let outsider_backend = DbBackend::new(pool.clone(), s.outsider.into());
        let err2 = outsider_backend
            .materialize_on_threshold(cmd())
            .await
            .unwrap_err();
        assert!(matches!(err2, TemperError::NotFound(_)), "unreadable → 404");
    }
}
