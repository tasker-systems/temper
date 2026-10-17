#![cfg(feature = "artifact-tests")]
//! `readback::invocation_show` / `invocation_list` — the agent-invocation envelope read binding. Proves:
//! a readable principal sees the envelope plus its stamped acts (SURFACE); a principal who cannot read
//! the originating cogmap gets `Ok(None)` / an empty list, NOT an error (DENY — leak-safe, like
//! `cogmap_shape`). The access gate lives INSIDE the SQL (`anchor_readable_by_profile`).

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::property_owner::PropertyOwner;
use temper_core::types::provenance::ProvenanceSource;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, fire_with, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{BlockId, CogmapId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{
    AgentAuthorship, AnchorRef, ConfidenceBand, EdgePolarity, Incorporation,
};
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
            owner: PropertyOwner::resource(ResourceId::from(telos)),
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

/// The reasoning text of each act, in order. An act's `metadata` is where the leak lived.
fn reasoning_of(row: &temper_substrate::readback::InvocationShowRow) -> Vec<String> {
    row.acts
        .iter()
        .filter_map(|a| a.metadata.get("reasoning").and_then(|r| r.as_str()))
        .map(str::to_string)
        .collect()
}

/// The leak this gate closes: reading the cogmap an invocation started from must not hand over
/// the reasoning an agent wrote about an element the reader cannot read. A cogmap member sees the
/// acts on what they can read and none of the acts that name a resource they cannot read — by its
/// home, by an edge pointing at it (first assert and re-assert alike), by a facet on that edge, or
/// after it is rehomed out of their reach (by owner and by block). A reader of both sees every act,
/// so the gate drops only what it must, and a soft delete does not hide the delete act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invocation_show_drops_acts_on_elements_the_reader_cannot_read(pool: PgPool) {
    common::seed_system(&pool).await;

    let (cogmap, telos) = common::genesis_cogmap(&pool, "inv-gate", "Invocation Gate").await;
    let entity = EntityId::from(system_entity(&pool).await);
    let system_profile = ProfileId::from(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM kb_profiles WHERE handle='system'")
            .fetch_one(&pool)
            .await
            .unwrap(),
    );

    // The map's team can read the cogmap and its own context; the hidden team owns a context only
    // the second reader belongs to.
    let map_team = common::create_team(&pool, "inv-gate-map").await;
    let hidden_team = common::create_team(&pool, "inv-gate-hidden").await;
    let map_only = common::create_profile(&pool, "map-only@example.com").await;
    let both = common::create_profile(&pool, "both@example.com").await;
    common::add_team_member(&pool, map_team, map_only).await;
    common::add_team_member(&pool, map_team, both).await;
    common::add_team_member(&pool, hidden_team, both).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(map_team)
        .bind(cogmap)
        .execute(&pool)
        .await
        .expect("join cogmap to its team");
    let hidden_context = ContextId::from(
        common::insert_context(&pool, "kb_teams", hidden_team, "hidden", "Hidden")
            .await
            .expect("insert the hidden team's context"),
    );
    let shared_context = ContextId::from(
        common::insert_context(&pool, "kb_teams", map_team, "shared", "Shared")
            .await
            .expect("insert the map team's context"),
    );

    // Created outside the invocation: a resource in the hidden context, one in the shared context
    // that the invocation deletes, and one in the shared context that is rehomed into the hidden one
    // after the invocation has acted on it.
    let mk = |home: ContextId, title: &'static str| {
        common::mk_embedded(
            &pool,
            AnchorRef::context(home),
            system_profile,
            entity,
            title,
            common::unit(0),
        )
    };
    let hidden = ResourceId::from(mk(hidden_context, "Hidden Resource").await);
    let doomed = ResourceId::from(mk(shared_context, "Doomed Resource").await);
    let moved = ResourceId::from(mk(shared_context, "Moved Resource").await);
    let moved_block: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1")
            .bind(moved.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

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

    let stamped = |reasoning: &str| EventContext {
        authorship: Some(AgentAuthorship {
            reasoning: Some(reasoning.to_string()),
            confidence: ConfidenceBand::Confident,
            rationale: None,
            persona: None,
            model: None,
        }),
        invocation: Some(invocation),
        correlation: None,
    };
    let values = json!({ "facet": "reviewed" });
    let assert_to_hidden = || SeedAction::RelationshipAssert {
        src: AnchorRef::resource(ResourceId::from(telos)),
        tgt: AnchorRef::resource(hidden),
        kind: EdgeKind::LeadsTo,
        polarity: EdgePolarity::Forward,
        label: Some("advances"),
        weight: 1.0,
        home: EdgeHome::Cogmap(CogmapId::from(cogmap)),
        emitter: entity,
    };
    let incorporated = [Incorporation {
        source: ProvenanceSource::Remote("https://example.com/cited".to_string()),
        seq: 0,
    }];

    let mut tx = pool.begin().await.unwrap();
    let facet = |owner: PropertyOwner| SeedAction::FacetSet {
        owner,
        values: &values,
        weight: 1.0,
        emitter: entity,
    };
    fire_with(
        &mut tx,
        facet(PropertyOwner::resource(ResourceId::from(telos))),
        stamped("telos: reads as shipped"),
    )
    .await
    .expect("facet on the telos");
    fire_with(
        &mut tx,
        facet(PropertyOwner::resource(hidden)),
        stamped("hidden: names a person"),
    )
    .await
    .expect("facet on the hidden resource");
    let edge = fire_with(&mut tx, assert_to_hidden(), stamped("edge: first assert"))
        .await
        .expect("edge to the hidden resource")
        .relationship()
        .unwrap();
    // The re-assert conflicts into the existing edge row, so its payload's edge_id never reaches
    // kb_edges: only the payload's own endpoints can gate it.
    fire_with(&mut tx, assert_to_hidden(), stamped("edge: re-assert"))
        .await
        .expect("re-assert the same edge");
    fire_with(
        &mut tx,
        facet(PropertyOwner::edge(edge)),
        stamped("edge: facet on it"),
    )
    .await
    .expect("facet on the edge");
    fire_with(
        &mut tx,
        SeedAction::ResourceDelete {
            resource: doomed,
            emitter: entity,
        },
        stamped("doomed: retired"),
    )
    .await
    .expect("delete the shared resource");
    fire_with(
        &mut tx,
        facet(PropertyOwner::resource(moved)),
        stamped("moved: by owner"),
    )
    .await
    .expect("facet on the resource that will move");
    fire_with(
        &mut tx,
        SeedAction::BlockAnnotate {
            block: BlockId::from(moved_block),
            incorporated: &incorporated,
            emitter: entity,
        },
        stamped("moved: by block"),
    )
    .await
    .expect("annotate the block of the resource that will move");
    tx.commit().await.unwrap();

    // Outside the invocation, the moved resource leaves the shared context for the hidden one. Its
    // two acts stay anchored where they happened (readable), so only the resource gate can drop them.
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceRehome {
            resource: moved,
            home: AnchorRef::context(hidden_context),
            emitter: entity,
        },
    )
    .await
    .expect("rehome into the hidden context");
    tx.commit().await.unwrap();

    let show = |reader: Uuid| {
        temper_substrate::readback::invocation_show(
            &pool,
            invocation.uuid(),
            ProfileId::from(reader),
        )
    };

    // The cogmap-only reader: the open, the telos act and the delete. Nothing that names the hidden
    // resource, however it is named.
    let shown = show(map_only)
        .await
        .expect("readable show")
        .expect("the envelope is visible to a cogmap reader");
    assert_eq!(
        shown
            .acts
            .iter()
            .map(|a| a.event_kind.as_str())
            .collect::<Vec<_>>(),
        ["delegated_launch", "property_asserted", "resource_deleted"],
        "only the open, the telos act and the delete surface for a cogmap-only reader: {:?}",
        shown.acts
    );
    assert_eq!(
        reasoning_of(&shown),
        ["telos: reads as shipped", "doomed: retired"],
        "no reasoning about the hidden resource reaches a cogmap-only reader"
    );

    // The reader of both: every act, so the gate removes only what the reader cannot read.
    let shown = show(both)
        .await
        .expect("readable show")
        .expect("the envelope is visible");
    let mut reasoning = reasoning_of(&shown);
    reasoning.sort();
    assert_eq!(
        reasoning,
        [
            "doomed: retired",
            "edge: facet on it",
            "edge: first assert",
            "edge: re-assert",
            "hidden: names a person",
            "moved: by block",
            "moved: by owner",
            "telos: reads as shipped",
        ],
        "a reader of the cogmap and the hidden context sees every act: {:?}",
        shown.acts
    );
}
