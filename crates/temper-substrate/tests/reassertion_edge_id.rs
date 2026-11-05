#![cfg(feature = "test-db")]
//! Witnesses for migration 20261117000030: a re-assertion of an active edge records the id of the
//! edge it updated, and the edge's trail reaches the re-assertions recorded before that migration.
//!
//! Before it, `fire` minted a fresh `edge_id` for every assertion. `_project_relationship_asserted`'s
//! ON CONFLICT then reweighted the existing edge and answered with ITS id, so the recorded event named
//! an edge with no row. Each witness names what makes it fail.

mod common;

use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_substrate::payloads::{self, AnchorRef, EdgePolarity};
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

fn chunk(prose: &str) -> IncomingChunk {
    use sha2::Digest;
    IncomingChunk {
        chunk_index: 0,
        content_hash: hex::encode(sha2::Sha256::digest(prose.trim())),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: "first".to_string(),
        heading_depth: 1,
    }
}

struct World {
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    task: ResourceId,
    goal: ResourceId,
}

async fn world(pool: &PgPool) -> World {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let emitter: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    let owner = ProfileId::from(profile);
    let home = ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), "reassert", "reassert")
            .await
            .unwrap(),
    );
    let mut ids = Vec::new();
    for title in ["a task", "a goal"] {
        let origin = format!("test://{title}");
        let body = format!("{title} body");
        ids.push(
            writes::create_resource_with(
                pool,
                CreateParams {
                    idempotency_key: None,
                    title,
                    origin_uri: &origin,
                    body: &body,
                    doc_type: "research",
                    home: AnchorRef::context(home),
                    owner,
                    originator: owner,
                    emitter,
                    properties: &[],
                    chunks: Some(vec![chunk(&body)]),
                    sources: vec![],
                },
                EventContext::default(),
            )
            .await
            .expect("create through the create path"),
        );
    }
    World {
        owner,
        emitter,
        home,
        task: ids[0],
        goal: ids[1],
    }
}

fn assert_action(w: &World, kind: EdgeKind, weight: f64) -> SeedAction<'static> {
    SeedAction::RelationshipAssert {
        src: AnchorRef::resource(w.task),
        tgt: AnchorRef::resource(w.goal),
        kind,
        polarity: EdgePolarity::Forward,
        label: Some("advances"),
        weight,
        home: EdgeHome::Context(w.home),
        emitter: w.emitter,
    }
}

async fn assert_edge(pool: &PgPool, w: &World, kind: EdgeKind, weight: f64) -> EdgeId {
    let mut tx = pool.begin().await.unwrap();
    let id = fire(&mut tx, assert_action(w, kind, weight))
        .await
        .expect("assert the edge")
        .relationship()
        .unwrap();
    tx.commit().await.unwrap();
    id
}

async fn fold(pool: &PgPool, w: &World, edge: EdgeId) {
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipFold {
            edge,
            reason: None,
            emitter: w.emitter,
        },
    )
    .await
    .expect("fold the edge");
    tx.commit().await.unwrap();
}

async fn retype(pool: &PgPool, w: &World, edge: EdgeId, kind: EdgeKind) {
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipRetype {
            edge,
            kind,
            polarity: EdgePolarity::Forward,
            emitter: w.emitter,
        },
    )
    .await
    .expect("retype the edge");
    tx.commit().await.unwrap();
}

/// Every `relationship_asserted` event's payload `edge_id`, in ledger order.
async fn asserted_edge_ids(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT (ev.payload->>'edge_id')::uuid FROM kb_events ev
           JOIN kb_event_types et ON et.id = ev.event_type_id
          WHERE et.name = 'relationship_asserted' ORDER BY ev.id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Record a re-assertion the way every version before 20261117000030 did: a freshly minted
/// `edge_id`, appended as written, then projected. The projector is unchanged, so its ON CONFLICT
/// still answers with the existing edge, and the event names an id with no row.
async fn historical_reassertion(pool: &PgPool, w: &World, kind: EdgeKind) -> (Uuid, Uuid) {
    let payload = payloads::RelationshipAsserted {
        edge_id: EdgeId::from(Uuid::now_v7()),
        source: AnchorRef::resource(w.task),
        target: AnchorRef::resource(w.goal),
        edge_kind: kind,
        polarity: EdgePolarity::Forward,
        label: Some("advances".into()),
        weight: 0.25,
        home: AnchorRef::context(w.home),
    };
    let payload = serde_json::to_value(&payload).unwrap();
    let mut tx = pool.begin().await.unwrap();
    let ev: Uuid = sqlx::query_scalar(
        "SELECT _event_append('relationship_asserted', $1, $2->'home'->>'table',
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
    let minted: Uuid = payload["edge_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(minted, answered, "the fixture must record a phantom");
    (ev, answered)
}

async fn trail(pool: &PgPool, w: &World, edge: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT event_id FROM element_trail_edge($1, $2)")
        .bind(w.owner.uuid())
        .bind(edge)
        .fetch_all(pool)
        .await
        .unwrap()
}

/// The task's witness. Asserting the same active edge twice records two events that both name the
/// one edge row, and that row carries the second weight.
///
/// FAILS IF: `relationship_assert` appends the payload as `fire` minted it. The second event then
/// names an id with no row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_reassertion_names_the_edge_it_updated(pool: PgPool) {
    let w = world(&pool).await;
    let first = assert_edge(&pool, &w, EdgeKind::LeadsTo, 1.0).await;
    let second = assert_edge(&pool, &w, EdgeKind::LeadsTo, 0.5).await;
    assert_eq!(
        first, second,
        "a re-assertion answers with the existing edge"
    );

    assert_eq!(
        asserted_edge_ids(&pool).await,
        vec![first.uuid(), first.uuid()],
        "both events name the one edge row"
    );
    let (rows, weight): (i64, f64) = sqlx::query_as(
        "SELECT count(*), max(weight) FROM kb_edges WHERE source_id = $1 AND target_id = $2",
    )
    .bind(w.task.uuid())
    .bind(w.goal.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((rows, weight), (1, 0.5), "one edge, reweighted");
    let orphans: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
          WHERE et.name = 'relationship_asserted'
            AND NOT EXISTS (SELECT 1 FROM kb_edges e WHERE e.id = (ev.payload->>'edge_id')::uuid)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        orphans, 0,
        "no relationship_asserted event names an edge with no row"
    );
}

/// Once an edge is folded, asserting the same identity makes a new edge, and the event names the
/// new edge, never the folded one.
///
/// FAILS IF: the lookup ignores `is_folded`. The event then names the folded edge, the projector
/// inserts a fresh row under that id, and the primary key refuses it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_assertion_after_a_fold_names_the_new_edge(pool: PgPool) {
    let w = world(&pool).await;
    let first = assert_edge(&pool, &w, EdgeKind::LeadsTo, 1.0).await;
    fold(&pool, &w, first).await;
    let second = assert_edge(&pool, &w, EdgeKind::LeadsTo, 1.0).await;
    assert_ne!(first, second, "a folded edge is not revived");
    assert_eq!(
        asserted_edge_ids(&pool).await,
        vec![first.uuid(), second.uuid()],
        "each event names the edge it made"
    );
}

/// Two first assertions of one identity, racing, both record the one edge that exists.
///
/// The first holds its transaction open while the second starts. Without the identity lock the
/// second reads no active edge and keeps its minted id. Its insert then waits on the first's row,
/// its ON CONFLICT answers with that row, and `relationship_assert` refuses the mismatch.
///
/// FAILS IF: the advisory lock is dropped. The second assertion then errors with 40001.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn racing_first_assertions_both_name_the_one_edge(pool: PgPool) {
    let w = world(&pool).await;
    let mut first_tx = pool.begin().await.unwrap();
    let first = fire(&mut first_tx, assert_action(&w, EdgeKind::LeadsTo, 1.0))
        .await
        .unwrap()
        .relationship()
        .unwrap();

    let pool2 = pool.clone();
    let action = assert_action(&w, EdgeKind::LeadsTo, 0.5);
    let second = tokio::spawn(async move {
        let mut tx = pool2.begin().await.unwrap();
        let id = fire(&mut tx, action).await?.relationship()?;
        tx.commit().await?;
        anyhow::Ok(id)
    });
    // Long enough for the second assertion to reach the lock or the index wait.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !second.is_finished(),
        "the second assertion waits on the first"
    );
    first_tx.commit().await.unwrap();

    let second = second.await.unwrap().expect("the second assertion commits");
    assert_eq!(first, second);
    assert_eq!(
        asserted_edge_ids(&pool).await,
        vec![first.uuid(), first.uuid()],
        "both events name the one edge row"
    );
}

/// The re-assertions recorded before the fix stay as written, and the edge's trail reaches them.
/// Each is attributed to the one edge that was active, with that kind, when it was recorded.
///
/// FAILS IF: `element_trail_edge` matches on `payload->>'edge_id'` alone (no phantom is on any
/// trail), or the phantom arm ignores the active window or the kind at the time (a phantom lands
/// on the wrong edge's trail).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_historical_reassertion_is_on_its_edges_trail(pool: PgPool) {
    let w = world(&pool).await;

    let e1 = assert_edge(&pool, &w, EdgeKind::LeadsTo, 1.0).await;
    let (p1, answered) = historical_reassertion(&pool, &w, EdgeKind::LeadsTo).await;
    assert_eq!(answered, e1.uuid());
    fold(&pool, &w, e1).await;

    let e2 = assert_edge(&pool, &w, EdgeKind::LeadsTo, 1.0).await;
    assert_ne!(e1, e2);
    let (p2, answered) = historical_reassertion(&pool, &w, EdgeKind::LeadsTo).await;
    assert_eq!(answered, e2.uuid());
    retype(&pool, &w, e2, EdgeKind::Near).await;
    let (p3, answered) = historical_reassertion(&pool, &w, EdgeKind::Near).await;
    assert_eq!(answered, e2.uuid());
    // e2 is now `near`, so the `leads_to` identity is free again: a third edge, active alongside e2,
    // with the same endpoints, home and label. Only the kind tells their re-assertions apart.
    let e3 = assert_edge(&pool, &w, EdgeKind::LeadsTo, 1.0).await;
    assert_ne!(e3, e2);
    let (p4, answered) = historical_reassertion(&pool, &w, EdgeKind::LeadsTo).await;
    assert_eq!(answered, e3.uuid());

    let t1 = trail(&pool, &w, e1.uuid()).await;
    let t2 = trail(&pool, &w, e2.uuid()).await;
    let t3 = trail(&pool, &w, e3.uuid()).await;
    assert!(t1.contains(&p1), "e1's trail holds the re-assertion of e1");
    assert!(
        !t1.contains(&p2) && !t1.contains(&p3) && !t1.contains(&p4),
        "and nothing recorded after e1 was folded"
    );
    assert!(
        t2.contains(&p2),
        "e2's trail holds its re-assertion before the retype"
    );
    assert!(t2.contains(&p3), "and the one after, under the new kind");
    assert!(!t2.contains(&p1), "and not e1's");
    assert!(
        !t2.contains(&p4),
        "and not e3's, recorded under the kind e2 no longer had"
    );
    assert_eq!(
        t3.iter()
            .filter(|e| [p1, p2, p3, p4].contains(e))
            .collect::<Vec<_>>(),
        vec![&p4],
        "e3's trail holds its own re-assertion and none of the others"
    );
}
