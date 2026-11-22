//! The builders the edge-erasure witnesses share: `edge_erasure_trail_test.rs`,
//! `edge_erasure_authority_test.rs`, `edge_erasure_unit_test.rs`, `edge_erasure_replay_test.rs`,
//! `sensitivity_edge_erasure_bridge_test.rs`, `person_erasure_estate_edges_test.rs` and
//! `write_lock_bound_test.rs`. One copy, so the seven build the same world: a profile whose `@me`
//! context is in its estate, resources homed there or in a team's context outside it, and edges,
//! facets and folds written through the real write paths. Profiles, entities, teams and contexts
//! come from `test_support`.

// `allow`, not `expect` (RUST-LINT-3's shared-module exception): this file is compiled into seven
// test binaries, each using a different subset of it (the authority witnesses never fold or
// facet; the lock-bound witness never replays), so `dead_code` fires in some binaries and not
// others, and an `expect` would be unfulfilled in whichever binary uses everything.
#![allow(
    dead_code,
    reason = "shared test module: each including binary uses a different subset"
)]

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::property_owner::PropertyOwner;
use temper_services::test_support;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, fire_with, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::{AgentAuthorship, AnchorRef, ConfidenceBand, EdgePolarity};
use temper_substrate::writes::{self, CreateParams};

pub struct World {
    pub profile: Uuid,
    pub emitter: Uuid,
    pub home: ContextId,
}

/// A profile, its `@web` emitter and an `@me` context, which `_erasure_estate_contexts(profile)`
/// returns. The handle is the full id, so same-millisecond UUIDv7 handles cannot collide.
pub async fn world(pool: &PgPool) -> World {
    let profile = Uuid::now_v7();
    let handle = format!("user-{profile}");
    test_support::profile::insert_with_id(pool, profile, &handle).await;
    let emitter = test_support::entity::insert(pool, profile, &format!("{handle}@web")).await;
    let home = test_support::context::insert_personal(pool, profile, "notes", "notes").await;
    let in_estate: bool = sqlx::query_scalar("SELECT $2 IN (SELECT _erasure_estate_contexts($1))")
        .bind(profile)
        .bind(home)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(in_estate, "the @me context is in its owner's estate");
    World {
        profile,
        emitter,
        home: ContextId::from(home),
    }
}

/// A context of a team that is not the world's personal team: outside the estate.
pub async fn team_context(pool: &PgPool) -> ContextId {
    let team = test_support::team::insert(pool, &format!("team-{}", Uuid::now_v7())).await;
    ContextId::from(test_support::context::insert_team(pool, team, "shared", "shared").await)
}

/// A resource homed in the world's `@me` context, through the real create path.
pub async fn resource(pool: &PgPool, w: &World, title: &str) -> ResourceId {
    resource_in(pool, w, w.home, title).await
}

/// A resource the world's profile owns, homed in `home`, through the real create path.
pub async fn resource_in(pool: &PgPool, w: &World, home: ContextId, title: &str) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{}", Uuid::now_v7()),
            body: "a body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: ProfileId::from(w.profile),
            originator: ProfileId::from(w.profile),
            emitter: EntityId::from(w.emitter),
            properties: &[],
            chunks: None,
            sources: Vec::new(),
        },
        EventContext::default(),
    )
    .await
    .expect("create through the real path")
}

/// An event context whose authorship carries `reasoning`, which the edge's trail then holds in
/// its metadata.
pub fn authored(reasoning: &str) -> EventContext {
    EventContext {
        authorship: Some(AgentAuthorship {
            reasoning: Some(reasoning.into()),
            confidence: ConfidenceBand::Probable,
            rationale: None,
            persona: None,
            model: None,
        }),
        invocation: None,
        correlation: None,
    }
}

/// A `leads_to` edge homed in the world's `@me` context, labelled `label`, asserted under `ctx`.
pub async fn edge(
    pool: &PgPool,
    w: &World,
    src: ResourceId,
    tgt: ResourceId,
    label: &str,
    ctx: EventContext,
) -> EdgeId {
    edge_in(pool, w, w.home, src, tgt, label, ctx).await
}

/// A `leads_to` edge homed in `home`, labelled `label`, asserted by the world's emitter under
/// `ctx`.
pub async fn edge_in(
    pool: &PgPool,
    w: &World,
    home: ContextId,
    src: ResourceId,
    tgt: ResourceId,
    label: &str,
    ctx: EventContext,
) -> EdgeId {
    let mut conn = pool.acquire().await.expect("acquire");
    fire_with(
        &mut conn,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(src),
            tgt: AnchorRef::resource(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(label),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter: EntityId::from(w.emitter),
        },
        ctx,
    )
    .await
    .expect("assert the edge")
    .relationship()
    .expect("an edge id")
}

/// A facet on the edge through the real write path; returns its `property_asserted` event.
pub async fn facet(pool: &PgPool, w: &World, e: EdgeId, values: Value) -> Uuid {
    let mut conn = pool.acquire().await.expect("acquire");
    fire(
        &mut conn,
        SeedAction::FacetSet {
            owner: PropertyOwner::Edge { id: e },
            values: &values,
            weight: 1.0,
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("set a facet on the edge");
    sqlx::query_scalar(
        "SELECT ev.id FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'property_asserted' \
            AND ev.payload #>> '{owner,table}' = 'kb_edges' \
            AND ev.payload #>> '{owner,id}' = $1::text \
          ORDER BY ev.id DESC LIMIT 1",
    )
    .bind(e.uuid())
    .fetch_one(pool)
    .await
    .expect("the facet's event")
}

/// The edge's ordinary fold, through the real write path.
pub async fn fold(pool: &PgPool, w: &World, e: EdgeId) {
    let mut conn = pool.acquire().await.expect("acquire");
    fire(
        &mut conn,
        SeedAction::RelationshipFold {
            edge: e,
            reason: None,
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("fold the edge");
}

/// A re-assertion as every version before 20261117000030 recorded one: the asserting event's
/// payload under a freshly minted `edge_id`, appended as written, then projected. The projector's
/// ON CONFLICT answers with the existing edge, so the event names an id with no row
/// (`_edge_phantom_reassertions`). Returns the re-assertion's event.
pub async fn historical_reassertion(pool: &PgPool, w: &World, e: EdgeId) -> Uuid {
    let asserted = asserting_event(pool, e).await;
    let payload: Value = sqlx::query_scalar(
        "SELECT jsonb_set(payload, '{edge_id}', to_jsonb($2::text)) FROM kb_events WHERE id = $1",
    )
    .bind(asserted)
    .bind(Uuid::now_v7())
    .fetch_one(pool)
    .await
    .expect("the re-assertion payload");
    let mut tx = pool.begin().await.unwrap();
    let ev: Uuid = sqlx::query_scalar(
        "SELECT _event_append('relationship_asserted', $1, $2->'home'->>'table', \
                              ($2->'home'->>'id')::uuid, $2)",
    )
    .bind(w.emitter)
    .bind(&payload)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let answered: Uuid = sqlx::query_scalar("SELECT _project_relationship_asserted($1, $2)")
        .bind(ev)
        .bind(&payload)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        answered,
        e.uuid(),
        "the projection reweights the existing edge"
    );
    let minted: Uuid = payload["edge_id"].as_str().unwrap().parse().unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_edges WHERE id = $1")
        .bind(minted)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "the fixture must record a phantom");
    ev
}

/// The edge's `asserted_by_event_id`.
pub async fn asserting_event(pool: &PgPool, e: EdgeId) -> Uuid {
    sqlx::query_scalar("SELECT asserted_by_event_id FROM kb_edges WHERE id = $1")
        .bind(e.uuid())
        .fetch_one(pool)
        .await
        .expect("the edge's asserting event")
}

/// Tombstone `profile` on `conn`, the column the person act's tombstone step sets. An edge
/// erasure's transaction commits only when its record's `estate_of` is tombstoned
/// (`edge_erasure_requires_an_erased_owner`), so every lawful witness tombstones the owner before
/// it commits.
pub async fn tombstone(conn: &mut sqlx::PgConnection, profile: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE kb_profiles SET tombstoned_at = COALESCE(tombstoned_at, now()) WHERE id = $1",
    )
    .bind(profile)
    .execute(conn)
    .await
    .map(|_| ())
}

/// `edge_erasure_execute` in one transaction: the unit, then `estate_of`'s tombstone when
/// `tombstoned` (as a person erasure tombstones its subject after its edge step), then the commit.
/// `Err` is the unit's own refusal, rolled back; `Ok` carries the unit's result and the commit's
/// refusal text, if any. Without the tombstone the commit is refused
/// (`edge_erasure_requires_an_erased_owner`).
pub async fn erase_edge(
    pool: &PgPool,
    e: EdgeId,
    estate_of: Uuid,
    operator: &World,
    request_ref: Uuid,
    tombstoned: bool,
) -> Result<(Value, Option<String>), String> {
    let mut tx = pool.begin().await.expect("begin the act");
    let out: Value = match sqlx::query_scalar("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(e.uuid())
        .bind(estate_of)
        .bind(operator.profile)
        .bind(operator.emitter)
        .bind(request_ref)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(out) => out,
        Err(err) => {
            tx.rollback().await.expect("roll back the refused act");
            return Err(err.to_string());
        }
    };
    if tombstoned {
        tombstone(&mut tx, estate_of)
            .await
            .expect("tombstone the estate's owner");
    }
    let refused = tx.commit().await.err().map(|err| err.to_string());
    Ok((out, refused))
}

/// Reset the schema in the current database to a clean, un-seeded baseline, for a replay witness
/// (the substrate's own helper lives in its test tree, which this crate cannot import).
pub async fn reset_namespace(pool: &PgPool) {
    use sqlx::Executor;
    // In batches, never one `DROP SCHEMA public CASCADE`: see `temper_substrate::test_schema`.
    temper_substrate::test_schema::drop_test_schemas(pool).await;
    pool.execute("CREATE SCHEMA public;")
        .await
        .expect("recreate the public schema");
    temper_substrate::MIGRATOR
        .run(pool)
        .await
        .expect("re-apply the migration chain");
    // One table per transaction: see `temper_substrate::test_schema::truncate_kb_tables`.
    temper_substrate::test_schema::truncate_kb_tables(pool).await;
}
