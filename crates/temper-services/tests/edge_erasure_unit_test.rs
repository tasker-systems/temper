#![cfg(feature = "test-db")]
//! Witnesses for the per-edge erasure unit `edge_erasure_execute` (20261201000060) and its
//! payload-free projection body `_edge_erasure_apply_redaction`.
//!
//! The unit erases one edge homed in a context of the named person's estate: it folds the edge
//! when it is live (one `relationship_folded`, reason `estate_erased`), appends one `edge_erased`
//! record, projects its `edge`-authority redaction rows, sets the edge's label to NULL and its
//! owned properties to the sentinels, and rewrites every event of the edge's trail to the
//! sentinels. Each world is built through the real write paths; the unit is called by SQL, as the
//! person act calls it. The operator and its emitter are a second profile's.

use serde_json::{json, Value};
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

const LABEL: &str = "jane roe told us";
const REASONING: &str = "jane roe said so on the call";
const OTHER_LABEL: &str = "another free label";
const OTHER_REASONING: &str = "the other edge's own reasoning";
/// Text the erased edge alone carries, in its label, its facet and its authorship metadata.
const NEEDLES: [&str; 4] = [LABEL, REASONING, "jane roe", "confided"];

struct World {
    profile: Uuid,
    emitter: Uuid,
    home: ContextId,
}

/// A profile, its `@web` emitter and an `@me` context, which `_erasure_estate_contexts(profile)`
/// returns. The handle is the full id, so same-millisecond UUIDv7 handles cannot collide.
async fn world(pool: &PgPool) -> World {
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

async fn resource(pool: &PgPool, w: &World, title: &str) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{}", Uuid::now_v7()),
            body: "a body",
            doc_type: "research",
            home: AnchorRef::context(w.home),
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

fn authored(reasoning: &str) -> EventContext {
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

/// An edge homed in the world's context, asserted with authorship reasoning on its event.
async fn edge(
    pool: &PgPool,
    w: &World,
    src: ResourceId,
    tgt: ResourceId,
    label: &str,
    reasoning: &str,
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
            home: EdgeHome::Context(w.home),
            emitter: EntityId::from(w.emitter),
        },
        authored(reasoning),
    )
    .await
    .expect("assert the edge")
    .relationship()
    .expect("an edge id")
}

/// A facet on the edge through the real write path.
async fn facet(pool: &PgPool, w: &World, e: EdgeId, values: Value) {
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
}

/// The edge's ordinary fold, through the real write path.
async fn fold(pool: &PgPool, w: &World, e: EdgeId) {
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

async fn asserting_event(pool: &PgPool, e: EdgeId) -> Uuid {
    sqlx::query_scalar("SELECT asserted_by_event_id FROM kb_edges WHERE id = $1")
        .bind(e.uuid())
        .fetch_one(pool)
        .await
        .expect("the edge's asserting event")
}

/// One estate with a labelled, faceted edge and a second edge beside it, and an operator.
struct Estate {
    w: World,
    op: World,
    e: EdgeId,
    asserted: Uuid,
    other: EdgeId,
}

async fn estate(pool: &PgPool) -> Estate {
    let w = world(pool).await;
    let op = world(pool).await;
    let a = resource(pool, &w, "alpha").await;
    let b = resource(pool, &w, "beta").await;
    let c = resource(pool, &w, "gamma").await;
    let e = edge(pool, &w, a, b, LABEL, REASONING).await;
    facet(
        pool,
        &w,
        e,
        json!({ "node_label": "jane roe", "status": "confided" }),
    )
    .await;
    let other = edge(pool, &w, b, c, OTHER_LABEL, OTHER_REASONING).await;
    let asserted = asserting_event(pool, e).await;
    Estate {
        w,
        op,
        e,
        asserted,
        other,
    }
}

/// `edge_erasure_execute` under the operator, in its own committed statement. Returns the unit's
/// result, or the refusal's text.
async fn unit(
    pool: &PgPool,
    x: &Estate,
    estate_of: Uuid,
    request_ref: Uuid,
) -> Result<Value, String> {
    sqlx::query_scalar("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(x.e.uuid())
        .bind(estate_of)
        .bind(x.op.profile)
        .bind(x.op.emitter)
        .bind(request_ref)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The `relationship_folded` events naming the edge: (payload, correlation id).
async fn folds_of(pool: &PgPool, e: EdgeId) -> Vec<(Value, Option<Uuid>)> {
    sqlx::query_as(
        "SELECT ev.payload, ev.correlation_id FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'relationship_folded' AND (ev.payload ->> 'edge_id')::uuid = $1 \
          ORDER BY ev.id",
    )
    .bind(e.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The `edge_erased` events naming the edge, by payload.
async fn records_of(pool: &PgPool, e: EdgeId) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT ev.payload FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'edge_erased' AND (ev.payload ->> 'subject_id')::uuid = $1 \
          ORDER BY ev.id",
    )
    .bind(e.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The edge is folded with no label, and every property row it owns is folded at the sentinels.
async fn assert_sentinelled(pool: &PgPool, e: EdgeId) {
    let (folded, label): (bool, Option<String>) =
        sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id = $1")
            .bind(e.uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(folded, "the edge is folded");
    assert_eq!(label, None, "the edge's label is NULL");
    let props: Vec<(String, Value, bool)> = sqlx::query_as(
        "SELECT property_key, property_value, is_folded FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1",
    )
    .bind(e.uuid())
    .fetch_all(pool)
    .await
    .unwrap();
    assert!(!props.is_empty(), "the fixture gave the edge properties");
    for (key, value, folded) in &props {
        let n = key
            .strip_prefix("erased-key-")
            .unwrap_or_else(|| panic!("key {key:?} is not a sentinel"));
        assert!(
            !n.is_empty() && !n.starts_with('0') && n.chars().all(|c| c.is_ascii_digit()),
            "key {key:?} is erased-key-<n>"
        );
        assert_eq!(*value, json!("erased"), "{key}: value at the sentinel");
        assert!(*folded, "{key}: the row is folded");
    }
}

/// No event's payload or metadata quotes the erased edge's text.
async fn assert_ledger_clean(pool: &PgPool) {
    for needle in NEEDLES {
        let hits: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_events \
              WHERE strpos(payload::text, $1) > 0 OR strpos(coalesce(metadata::text, ''), $1) > 0",
        )
        .bind(needle)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(hits, 0, "the ledger still quotes {needle:?}");
    }
}

/// FAILS IF the unit leaves the edge live or labelled, a property row it owns off the sentinels or
/// live, any ledger copy of its text, a redaction row under another authority, a record that does
/// not name the edge, the estate and its own fold, or a fold with another reason; or if it reaches
/// the edge beside it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unit_folds_and_sentinels_one_edge(pool: PgPool) {
    let x = estate(&pool).await;
    let request_ref = Uuid::now_v7();
    let out = unit(&pool, &x, x.w.profile, request_ref)
        .await
        .expect("the unit erases the edge");

    assert_eq!(out["folded"], json!(true), "the unit folded the live edge");
    let record_id: Uuid = out["event_id"]
        .as_str()
        .expect("the record's id")
        .parse()
        .unwrap();

    assert_sentinelled(&pool, x.e).await;
    assert_ledger_clean(&pool).await;

    let records = records_of(&pool, x.e).await;
    assert_eq!(records.len(), 1, "one edge_erased record");
    let record = &records[0];
    assert_eq!(record["subject_table"], json!("kb_edges"));
    assert_eq!(record["subject_id"], json!(x.e.uuid()));
    assert_eq!(record["estate_of"], json!(x.w.profile));
    assert_eq!(record["actor"], json!(x.op.profile));
    assert_eq!(record["folded"], json!(true));
    assert_eq!(
        record["redacted_fields"], out["redacted_fields"],
        "the unit returns the record's redacted fields"
    );
    let named: Vec<Uuid> = record["redacted_fields"]
        .as_array()
        .expect("redacted_fields")
        .iter()
        .map(|f| f["event"].as_str().unwrap().parse().unwrap())
        .collect();
    assert!(
        named.contains(&x.asserted),
        "the record names the edge's asserting event"
    );

    let correlated: Option<Uuid> =
        sqlx::query_scalar("SELECT correlation_id FROM kb_events WHERE id = $1")
            .bind(record_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        correlated,
        Some(request_ref),
        "the record carries the request"
    );

    let folds = folds_of(&pool, x.e).await;
    assert_eq!(folds.len(), 1, "one fold of the edge");
    assert_eq!(folds[0].0["reason"], json!("estate_erased"));
    assert_eq!(
        folds[0].1,
        Some(request_ref),
        "the fold carries the request"
    );

    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT event_id, authority FROM kb_event_field_redactions WHERE redacted_by = $1",
    )
    .bind(record_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(!rows.is_empty(), "the record projected its redaction rows");
    for (event, authority) in &rows {
        assert_eq!(authority, "edge", "event {event}: the row's authority");
    }
    assert!(rows.iter().any(|(ev, _)| *ev == x.asserted));

    let (folded, label): (bool, Option<String>) =
        sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id = $1")
            .bind(x.other.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!folded, "the edge beside it stays live");
    assert_eq!(label.as_deref(), Some(OTHER_LABEL), "and keeps its label");
    let (payload_label, reasoning): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT payload ->> 'label', metadata ->> 'reasoning' FROM kb_events WHERE id = $1",
    )
    .bind(asserting_event(&pool, x.other).await)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        payload_label.as_deref(),
        Some(OTHER_LABEL),
        "the edge beside it keeps its ledger label"
    );
    assert_eq!(
        reasoning.as_deref(),
        Some(OTHER_REASONING),
        "and its authorship reasoning"
    );
}

/// FAILS IF a second run over an erased edge appends anything, or answers other than `already
/// erased`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_second_run_is_refused_already_erased(pool: PgPool) {
    let x = estate(&pool).await;
    unit(&pool, &x, x.w.profile, Uuid::now_v7())
        .await
        .expect("the first run erases the edge");
    let before = event_count(&pool).await;
    let err = unit(&pool, &x, x.w.profile, Uuid::now_v7())
        .await
        .expect_err("a second run is refused");
    assert!(
        err.contains("edge_erasure_execute: already erased"),
        "refused as already erased; got {err}"
    );
    assert_eq!(event_count(&pool).await, before, "nothing appended");
    assert_eq!(records_of(&pool, x.e).await.len(), 1, "one record");
}

/// FAILS IF the unit erases an edge whose home is not in the estate it names, or appends anything
/// before refusing it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_outside_the_named_estate_is_refused(pool: PgPool) {
    let x = estate(&pool).await;
    let stranger = world(&pool).await;
    let before = event_count(&pool).await;
    let err = unit(&pool, &x, stranger.profile, Uuid::now_v7())
        .await
        .expect_err("an edge outside the named estate is refused");
    assert!(
        err.contains(&format!(
            "edge_erasure_execute: edge {} is not homed in the estate of {}",
            x.e.uuid(),
            stranger.profile
        )),
        "refused for its home; got {err}"
    );
    assert_eq!(event_count(&pool).await, before, "nothing appended");
    let (folded, label): (bool, Option<String>) =
        sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id = $1")
            .bind(x.e.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!folded, "the edge stays live");
    assert_eq!(label.as_deref(), Some(LABEL), "and keeps its label");
}

/// FAILS IF the unit refuses an edge already folded by the ordinary path, folds it a second time,
/// claims the fold in its record, or leaves its label, properties or ledger text in place.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_already_folded_edge_is_erased_without_a_new_fold(pool: PgPool) {
    let x = estate(&pool).await;
    fold(&pool, &x.w, x.e).await;
    assert_eq!(folds_of(&pool, x.e).await.len(), 1, "the ordinary fold");

    let out = unit(&pool, &x, x.w.profile, Uuid::now_v7())
        .await
        .expect("the unit erases a folded edge");
    assert_eq!(out["folded"], json!(false), "the unit did not fold it");

    let folds = folds_of(&pool, x.e).await;
    assert_eq!(folds.len(), 1, "no second fold");
    assert_ne!(
        folds[0].0["reason"],
        json!("estate_erased"),
        "the one fold is the ordinary one"
    );
    let records = records_of(&pool, x.e).await;
    assert_eq!(records.len(), 1, "one edge_erased record");
    assert_eq!(records[0]["folded"], json!(false));

    assert_sentinelled(&pool, x.e).await;
    assert_ledger_clean(&pool).await;
}

/// A facet on the edge, written on a connection the caller holds, returning the write's error.
async fn facet_on(
    conn: &mut sqlx::PgConnection,
    w: &World,
    e: EdgeId,
    values: &Value,
) -> anyhow::Result<()> {
    fire(
        conn,
        SeedAction::FacetSet {
            owner: PropertyOwner::Edge { id: e },
            values,
            weight: 1.0,
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .map(|_| ())
}

/// The SQLSTATE of the first database error in an error's chain.
fn sqlstate(err: &anyhow::Error) -> Option<String> {
    err.chain()
        .find_map(|c| c.downcast_ref::<sqlx::Error>())
        .and_then(|e| e.as_database_error())
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
}

/// A facet writer of the edge while the unit's transaction is open waits on the unit's lock of the
/// edge row; once the act commits, the same write is refused, and nothing it carried reaches the
/// projection or the ledger.
///
/// FAILS IF the write guard takes no lock on the `kb_edges` row (the first write lands under the
/// open unit instead of timing out with `55P03`, as it does when only the edge's live resource
/// endpoints are locked), or if a write to the folded, recorded edge is admitted after the act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_property_write_racing_the_unit_waits_and_then_is_refused(pool: PgPool) {
    let x = estate(&pool).await;
    // A fresh inner key: no row of the edge holds it, so the write touches no kb_properties row the
    // unit rewrote and can wait only on the edge row.
    let late = json!({ "witness": "a later mark" });

    let mut act = pool.begin().await.expect("begin the unit's transaction");
    sqlx::query("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(x.e.uuid())
        .bind(x.w.profile)
        .bind(x.op.profile)
        .bind(x.op.emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *act)
        .await
        .expect("the unit runs, its transaction held open");

    let mut writer = pool.begin().await.expect("begin the writer's transaction");
    sqlx::query("SET LOCAL lock_timeout = '500ms'")
        .execute(&mut *writer)
        .await
        .unwrap();
    let err = facet_on(&mut writer, &x.w, x.e, &late)
        .await
        .expect_err("the writer waits on the open unit");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("55P03"),
        "the writer timed out waiting for a lock; got {err:#}"
    );
    writer.rollback().await.unwrap();

    act.commit().await.expect("the act commits");

    let before = event_count(&pool).await;
    let mut conn = pool.acquire().await.expect("acquire");
    let err = facet_on(&mut conn, &x.w, x.e, &late)
        .await
        .expect_err("a write to the erased edge is refused");
    assert!(
        format!("{err:#}").contains(&format!(
            "edge {} is erased; writes are refused",
            x.e.uuid()
        )),
        "refused as erased; got {err:#}"
    );
    assert_eq!(event_count(&pool).await, before, "nothing appended");

    assert_sentinelled(&pool, x.e).await;
    assert_ledger_clean(&pool).await;
    let hits: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM kb_events \
                  WHERE strpos(payload::text, 'a later mark') > 0) \
              + (SELECT count(*) FROM kb_properties \
                  WHERE strpos(property_value::text, 'a later mark') > 0)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hits, 0, "the refused write left nothing");
}
