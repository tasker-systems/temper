#![cfg(feature = "test-db")]
//! L0 kernel cognitive map: the public, root-team-joined system-default cogmap,
//! born deterministically by migration 20260625000001 via cogmap_genesis.

use sqlx::PgPool;
use uuid::Uuid;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);
const L0_TELOS: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000002);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_cogmap_is_born_at_migration(pool: PgPool) {
    // The L0 cogmap exists with the reserved id, the canonical name, and its telos.
    let (name, telos): (String, Uuid) =
        sqlx::query_as("SELECT name, telos_resource_id FROM kb_cogmaps WHERE id = $1")
            .bind(L0_COGMAP)
            .fetch_one(&pool)
            .await
            .expect("L0 cogmap must exist after migrations");
    assert_eq!(name, "system-default");
    assert_eq!(telos, L0_TELOS);

    // Its telos resource exists and is stamped doc_type = cogmap_charter (genesis does this).
    let (title,): (String,) = sqlx::query_as("SELECT title FROM kb_resources WHERE id = $1")
        .bind(L0_TELOS)
        .fetch_one(&pool)
        .await
        .expect("L0 telos resource must exist");
    assert_eq!(title, "What Temper Is");

    let doc_type: serde_json::Value = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
         WHERE owner_table = 'kb_resources' AND owner_id = $1 AND property_key = 'doc_type'",
    )
    .bind(L0_TELOS)
    .fetch_one(&pool)
    .await
    .expect("L0 telos must have a doc_type property");
    assert_eq!(doc_type, serde_json::json!("cogmap_charter"));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_is_event_sourced_and_homed(pool: PgPool) {
    // Exactly one cogmap_seeded event, emitted by the system entity, producing L0.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'cogmap_seeded' \
            AND ev.producing_anchor_table = 'kb_cogmaps' \
            AND ev.producing_anchor_id = $1",
    )
    .bind(L0_COGMAP)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        count, 1,
        "L0 must be born via exactly one cogmap_seeded event"
    );

    // The telos resource is homed in L0 (anchor_table = kb_cogmaps).
    let (anchor_table, anchor_id): (String, Uuid) = sqlx::query_as(
        "SELECT anchor_table, anchor_id FROM kb_resource_homes WHERE resource_id = $1",
    )
    .bind(L0_TELOS)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(anchor_table, "kb_cogmaps");
    assert_eq!(anchor_id, L0_COGMAP);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_joined_only_to_root_team(pool: PgPool) {
    // L0 is joined to exactly one team, and that team is temper-system.
    let slugs: Vec<String> = sqlx::query_scalar(
        "SELECT t.slug FROM kb_team_cogmaps tc JOIN kb_teams t ON t.id = tc.team_id \
          WHERE tc.cogmap_id = $1 ORDER BY t.slug",
    )
    .bind(L0_COGMAP)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(slugs, vec!["temper-system".to_string()]);

    // cogmaps_share_a_team is reflexive for L0 (sanity: the access predicate sees the join).
    let shares: bool = sqlx::query_scalar("SELECT cogmaps_share_a_team($1, $1)")
        .bind(L0_COGMAP)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        shares,
        "L0 must share a team with itself once joined to temper-system"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_genesis_guard_is_idempotent(pool: PgPool) {
    // Re-running the guarded genesis block must NOT create a second L0 cogmap.
    // (Simulates a logical re-application of the seed's body.)
    sqlx::query(
        "DO $$ DECLARE v_emitter uuid := (SELECT e.id FROM kb_entities e \
              JOIN kb_profiles p ON p.id = e.profile_id \
             WHERE p.handle='system' AND e.name='system'); \
            v_owner uuid := (SELECT id FROM kb_profiles WHERE handle='system'); \
         BEGIN \
            IF NOT EXISTS (SELECT 1 FROM kb_cogmaps WHERE id='00000000-0000-0000-0005-000000000001') THEN \
               PERFORM cogmap_genesis( \
                 jsonb_build_object('cogmap_id','00000000-0000-0000-0005-000000000001', \
                   'name','system-default','owner_profile_id',v_owner, \
                   'telos', jsonb_build_object('resource_id','00000000-0000-0000-0005-000000000002', \
                     'title','What Temper Is','origin_uri','temper://system/what-is-temper', \
                     'blocks','[]'::jsonb)), '{}'::jsonb, v_emitter); \
            END IF; \
         END $$;",
    )
    .execute(&pool)
    .await
    .unwrap();

    let cogmaps: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_cogmaps WHERE id = $1")
        .bind(L0_COGMAP)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        cogmaps, 1,
        "re-running the guarded genesis must not duplicate L0"
    );
}
