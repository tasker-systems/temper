#![cfg(feature = "artifact-tests")]
//! `readback::invocation_show` / `invocation_list` — the agent-invocation envelope read binding. Proves:
//! a readable principal sees the envelope plus its stamped acts (SURFACE); a principal who cannot read
//! the originating cogmap gets `Ok(None)` / an empty list, NOT an error (DENY — leak-safe, like
//! `cogmap_shape`). The access gate lives INSIDE the SQL (`anchor_readable_by_profile`).

use serde_json::json;
use sqlx::PgPool;
use temper_substrate::events::{fire_with, EventContext, SeedAction};
use temper_substrate::ids::{CogmapId, EntityId, ProfileId, ResourceId};
use temper_substrate::writes::{self, OpenParams};
use uuid::Uuid;

mod common;

/// The boot-seeded canonical `system` entity id (emitter for the seed fires).
async fn system_entity(pool: &PgPool) -> Uuid {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
        .bind(profile)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invocation_show_and_list_surface_for_reader_and_gate_outsider(pool: PgPool) {
    common::seed_system(&pool).await; // boot the canonical `system` actor (see common/mod.rs)

    // Genesis a cogmap (creates the cogmap + telos resource + events).
    let (cogmap, telos) = common::genesis_cogmap(&pool, "inv-test", "Invocation Test").await;
    let entity = EntityId::from(system_entity(&pool).await);

    // A fresh NON-root team + two profiles: P1 a member (readable), P2 not (denied) — exactly the
    // cogmap_shape readability template.
    let team = common::create_team(&pool, "inv-team").await;
    let p1 = common::create_profile(&pool, "member@example.com").await;
    let p2 = common::create_profile(&pool, "outsider@example.com").await;
    common::add_team_member(&pool, team, p1).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(&pool)
        .await
        .expect("join cogmap to team");

    // Open the invocation envelope against the cogmap, minting its id.
    let invocation = writes::open_invocation(
        &pool,
        OpenParams {
            trigger_kind: "agent_run".to_string(),
            originating: CogmapId::from(cogmap),
            parent: None,
            scoped_entity: entity,
            emitter: entity,
        },
    )
    .await
    .expect("open invocation");

    // Fire ONE act stamped with the invocation id (the facet_set arm threads `EventContext.invocation`
    // → kb_events.invocation_id). The act is set on the cogmap's telos resource.
    let values = json!({ "facet": "shipped" });
    let mut tx = pool.begin().await.unwrap();
    fire_with(
        &mut tx,
        SeedAction::FacetSet {
            resource: ResourceId::from(telos),
            values: &values,
            weight: 1.0,
            emitter: entity,
        },
        EventContext {
            authorship: None,
            invocation: Some(invocation),
            correlation: None,
        },
    )
    .await
    .expect("fire stamped act");
    tx.commit().await.unwrap();

    // SURFACE — the reader sees the envelope plus its one stamped act.
    let shown =
        temper_substrate::readback::invocation_show(&pool, invocation.uuid(), ProfileId::from(p1))
            .await
            .expect("readable show")
            .expect("envelope present for reader");
    assert_eq!(shown.id, invocation.uuid());
    assert_eq!(shown.status, "open");
    assert_eq!(shown.trigger_kind, "agent_run");
    assert_eq!(shown.originating_cogmap_id, cogmap);
    assert_eq!(shown.parent_cogmap_id, None);
    assert_eq!(shown.telos_resource_id, telos);
    assert_eq!(shown.outcome, None);
    assert_eq!(shown.closed_at, None);
    // The open itself (`delegated_launch`) is stamped with the invocation id, so both it and the
    // facet act fold under the envelope — ordered by occurred_at (open first).
    let kinds: Vec<&str> = shown.acts.iter().map(|a| a.event_kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["delegated_launch", "property_asserted"],
        "the open + the stamped act surface, oldest first: {:?}",
        shown.acts
    );
    assert!(
        shown
            .acts
            .iter()
            .all(|a| a.emitter_entity_id == entity.uuid()),
        "every act carries the system emitter: {:?}",
        shown.acts
    );

    // SURFACE — the list read returns the envelope for the reader.
    let listed =
        temper_substrate::readback::invocation_list(&pool, ProfileId::from(p1), None, None)
            .await
            .expect("readable list");
    assert_eq!(
        listed.len(),
        1,
        "reader sees the one invocation: {listed:?}"
    );
    assert_eq!(listed[0].id, invocation.uuid());

    // DENY — a principal who cannot read the originating cogmap gets None (NOT an error).
    let denied =
        temper_substrate::readback::invocation_show(&pool, invocation.uuid(), ProfileId::from(p2))
            .await
            .expect("gate denial is None, not an error");
    assert!(denied.is_none(), "outsider must not see the envelope");

    // DENY — the list is empty for the outsider (no act leak either).
    let denied_list =
        temper_substrate::readback::invocation_list(&pool, ProfileId::from(p2), None, None)
            .await
            .expect("gate denial is empty, not an error");
    assert!(
        denied_list.is_empty(),
        "outsider must see no invocations: {denied_list:?}"
    );
}
