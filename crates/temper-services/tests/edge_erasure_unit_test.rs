#![cfg(feature = "test-db")]
//! Witnesses for the per-edge erasure unit `edge_erasure_execute` (20261201000060) and its
//! payload-free projection body `_edge_erasure_apply_redaction`.
//!
//! The unit erases one edge homed in a context of the named person's estate: it folds the edge
//! when it is live (one `relationship_folded`, reason `estate_erased`), appends one `edge_erased`
//! record, projects its `edge`-authority redaction rows, sets the edge's label to NULL and its
//! owned properties to the sentinels, and rewrites every event of the edge's trail to the
//! sentinels. Each world is built through the real write paths; the unit is called by SQL
//! directly, so each witness isolates the unit. The person act calls it for every edge homed in
//! its estate; `person_erasure_estate_edges_test` witnesses it through that act. The operator and
//! its emitter are a second profile's. A lawful run tombstones the estate's owner in the unit's
//! transaction before it commits, as the person act does after its edge step: the owner is
//! checked at commit (`edge_erasure_requires_an_erased_owner`).

mod edge_erasure;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, SeedAction};
use temper_substrate::ids::{EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};

use edge_erasure::{
    asserting_event, authored, edge, erase_edge, facet, fold, resource, tombstone, world, World,
};

const LABEL: &str = "jane roe told us";
const REASONING: &str = "jane roe said so on the call";
const OTHER_LABEL: &str = "another free label";
const OTHER_REASONING: &str = "the other edge's own reasoning";
/// Text the erased edge alone carries, in its label, its facet and its authorship metadata.
const NEEDLES: [&str; 4] = [LABEL, REASONING, "jane roe", "confided"];

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
    let e = edge(pool, &w, a, b, LABEL, authored(REASONING)).await;
    facet(
        pool,
        &w,
        e,
        json!({ "node_label": "jane roe", "status": "confided" }),
    )
    .await;
    let other = edge(pool, &w, b, c, OTHER_LABEL, authored(OTHER_REASONING)).await;
    let asserted = asserting_event(pool, e).await;
    Estate {
        w,
        op,
        e,
        asserted,
        other,
    }
}

/// `edge_erasure_execute` under the operator, as a person erasure runs it: `estate_of` is
/// tombstoned in the unit's transaction before it commits ([`erase_edge`]). Returns the unit's
/// result, or the refusal's text, the unit's own or the commit's.
async fn unit(
    pool: &PgPool,
    x: &Estate,
    estate_of: Uuid,
    request_ref: Uuid,
) -> Result<Value, String> {
    match erase_edge(pool, x.e, estate_of, &x.op, request_ref, true).await? {
        (out, None) => Ok(out),
        (_, Some(refused)) => Err(refused),
    }
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

/// The edge is re-homed by SQL to `kb_cogmaps` inside the probe's transaction, its
/// `home_anchor_id` kept at the estate's context, so only the home-table arm can refuse it; the
/// same unit on the unchanged edge is the admitted twin.
///
/// FAILS IF the unit erases an edge whose home is not a context because its anchor id matches one
/// of the estate's contexts, or appends anything before refusing it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unit_refuses_an_edge_not_homed_in_a_context(pool: PgPool) {
    let x = estate(&pool).await;
    let before = event_count(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE kb_edges SET home_anchor_table = 'kb_cogmaps' WHERE id = $1")
        .bind(x.e.uuid())
        .execute(&mut *tx)
        .await
        .expect("re-home the edge's table in the probe's transaction");
    let err = sqlx::query_scalar::<_, Value>("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(x.e.uuid())
        .bind(x.w.profile)
        .bind(x.op.profile)
        .bind(x.op.emitter)
        .bind(Uuid::now_v7())
        .fetch_one(&mut *tx)
        .await
        .expect_err("an edge homed on a cogmap is refused")
        .to_string();
    tx.rollback().await.unwrap();
    assert!(
        err.contains(&format!(
            "edge_erasure_execute: edge {} is not homed in the estate of {}",
            x.e.uuid(),
            x.w.profile
        )),
        "refused for its home table; got {err}"
    );
    assert_eq!(event_count(&pool).await, before, "nothing appended");

    let out = unit(&pool, &x, x.w.profile, Uuid::now_v7())
        .await
        .expect("the twin: the same unit on the context-homed edge erases it");
    assert_eq!(out["folded"], json!(true));
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

/// A write of the edge the race witnesses make, on a connection the caller holds.
#[derive(Debug, Clone, Copy)]
enum Write {
    /// A facet with an inner key no row of the edge holds, so the write touches no
    /// `kb_properties` row the unit rewrote and can wait only on the edge row.
    Facet,
    /// The ordinary fold, with a free-text reason.
    Fold,
}

/// The text each write carries, which must reach neither the projection nor the ledger.
const LATE: &str = "a later mark";

impl Write {
    /// The statement text the write's blocked backend runs, for [`await_blocked`].
    fn statement(self) -> &'static str {
        match self {
            Write::Facet => "%facet_set%",
            Write::Fold => "%relationship_fold%",
        }
    }

    async fn run(
        self,
        conn: &mut sqlx::PgConnection,
        emitter: Uuid,
        e: EdgeId,
    ) -> anyhow::Result<()> {
        let values = json!({ "witness": LATE });
        let action = match self {
            Write::Facet => SeedAction::FacetSet {
                owner: PropertyOwner::Edge { id: e },
                values: &values,
                weight: 1.0,
                emitter: EntityId::from(emitter),
            },
            Write::Fold => SeedAction::RelationshipFold {
                edge: e,
                reason: Some(LATE),
                emitter: EntityId::from(emitter),
            },
        };
        fire(conn, action).await.map(|_| ())
    }
}

/// The SQLSTATE of the first database error in an error's chain.
fn sqlstate(err: &anyhow::Error) -> Option<String> {
    err.chain()
        .find_map(|c| c.downcast_ref::<sqlx::Error>())
        .and_then(|e| e.as_database_error())
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
}

/// Wait until a backend running `statement` is blocked on a lock `holder` holds. Panics if none is
/// within ten seconds, or if `writer` finished first, which means it never waited.
async fn await_blocked<T>(
    pool: &PgPool,
    holder: i32,
    statement: &str,
    writer: &tokio::task::JoinHandle<T>,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)) AND query ILIKE $2)",
        )
        .bind(holder)
        .bind(statement)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(
            !writer.is_finished(),
            "the write finished while the unit held the edge: it did not wait on the unit"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "the write never blocked on the unit"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The unit runs in a transaction left open; `write` starts on its own connection, is seen blocked
/// on the unit's lock, and resumes when the act commits. The resumed write must be refused, and
/// nothing it carried may reach the projection or the ledger. A second write after the commit is
/// refused the same way.
async fn race(pool: &PgPool, write: Write) {
    let x = estate(pool).await;

    let mut act = pool.begin().await.expect("begin the unit's transaction");
    let holder: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *act)
        .await
        .unwrap();
    sqlx::query("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(x.e.uuid())
        .bind(x.w.profile)
        .bind(x.op.profile)
        .bind(x.op.emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *act)
        .await
        .expect("the unit runs, its transaction held open");
    tombstone(&mut act, x.w.profile)
        .await
        .expect("the person act's tombstone, after its edge step");

    let mut conn = pool.acquire().await.expect("the writer's connection");
    let (emitter, e) = (x.w.emitter, x.e);
    let writer = tokio::spawn(async move { write.run(&mut conn, emitter, e).await });
    await_blocked(pool, holder, write.statement(), &writer).await;

    act.commit().await.expect("the act commits");
    let before = event_count(pool).await;
    let refused = format!("edge {} is erased; writes are refused", x.e.uuid());
    let err = writer
        .await
        .expect("the writer's task")
        .expect_err("the write that waited is refused when it resumes");
    assert!(
        format!("{err:#}").contains(&refused),
        "{write:?} that waited: refused as erased; got {err:#}"
    );

    let mut conn = pool.acquire().await.expect("acquire");
    let err = write
        .run(&mut conn, x.w.emitter, x.e)
        .await
        .expect_err("a write after the act is refused");
    assert!(
        format!("{err:#}").contains(&refused),
        "{write:?} after the act: refused as erased; got {err:#}"
    );
    assert_eq!(event_count(pool).await, before, "nothing appended");

    assert_sentinelled(pool, x.e).await;
    assert_ledger_clean(pool).await;
    let hits: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM kb_events \
                  WHERE strpos(payload::text, $1) > 0) \
              + (SELECT count(*) FROM kb_properties \
                  WHERE strpos(property_value::text, $1) > 0)",
    )
    .bind(LATE)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(hits, 0, "the refused writes left nothing");
    let folds = folds_of(pool, x.e).await;
    assert_eq!(folds.len(), 1, "the unit's fold alone");
}

/// FAILS IF the write guard takes no lock on the `kb_edges` row (the facet write lands while the
/// unit is open: as when only the edge's live resource endpoints are locked, it never blocks); if
/// the lock and the check are one statement (the resumed write reads the record in its pre-wait
/// snapshot, misses it, and is admitted); or if a facet write to the erased edge is admitted.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_property_write_racing_the_unit_waits_and_then_is_refused(pool: PgPool) {
    race(&pool, Write::Facet).await;

    // Bounded, the same wait surfaces as a lock timeout while the unit is open.
    let x = estate(&pool).await;
    let mut act = pool.begin().await.unwrap();
    sqlx::query("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(x.e.uuid())
        .bind(x.w.profile)
        .bind(x.op.profile)
        .bind(x.op.emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *act)
        .await
        .unwrap();
    let mut writer = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '500ms'")
        .execute(&mut *writer)
        .await
        .unwrap();
    let err = Write::Facet
        .run(&mut writer, x.w.emitter, x.e)
        .await
        .expect_err("the bounded writer times out on the open unit");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("55P03"),
        "the writer timed out waiting for a lock; got {err:#}"
    );
    writer.rollback().await.unwrap();
    act.rollback().await.unwrap();
}

/// FAILS IF `_project_relationship_folded` admits a fold of an edge an `edge_erased` record names:
/// the fold blocks on the unit's row lock either way, and without the check it resumes after the
/// act, commits, and leaves its free-text reason on the erased edge's ledger trail.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_fold_racing_the_unit_waits_and_then_is_refused(pool: PgPool) {
    race(&pool, Write::Fold).await;
}

/// The unit runs in a transaction left open; an assertion of the erased edge's identity (its
/// endpoints, kind, polarity, label and home: what `uq_kb_edges_assertion` keys on) starts on its
/// own connection, carrying no authorship, and is seen blocked in `relationship_assert`'s
/// active-edge lookup on the unit's row lock. When the act commits, the lookup's re-check finds
/// the edge folded with no label and answers none, so the assertion mints a new live edge. That is
/// a fresh write of the asserter's own text, not the erased edge coming back
/// (20261201000060's header).
///
/// FAILS IF the assertion never waits on the unit; if it is refused; if it reweights, unfolds or
/// relabels the erased edge instead of inserting another; if the new edge is not live at the same
/// home with the asserter's label, under its own asserting event that names it; or if the new
/// event joins the erased edge's record, its redaction rows or the erased predicate, or brings
/// back any text the asserter did not send.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_reassertion_racing_the_unit_waits_and_then_mints_a_new_edge(pool: PgPool) {
    let x = estate(&pool).await;
    let (src, tgt): (Uuid, Uuid) =
        sqlx::query_as("SELECT source_id, target_id FROM kb_edges WHERE id = $1")
            .bind(x.e.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    let mut act = pool.begin().await.expect("begin the unit's transaction");
    let holder: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *act)
        .await
        .unwrap();
    let out: Value = sqlx::query_scalar("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(x.e.uuid())
        .bind(x.w.profile)
        .bind(x.op.profile)
        .bind(x.op.emitter)
        .bind(Uuid::now_v7())
        .fetch_one(&mut *act)
        .await
        .expect("the unit runs, its transaction held open");
    tombstone(&mut act, x.w.profile)
        .await
        .expect("the person act's tombstone, after its edge step");

    let mut conn = pool.acquire().await.expect("the asserter's connection");
    let (emitter, home) = (x.w.emitter, x.w.home);
    let asserter = tokio::spawn(async move {
        fire(
            &mut conn,
            SeedAction::RelationshipAssert {
                src: AnchorRef::resource(ResourceId::from(src)),
                tgt: AnchorRef::resource(ResourceId::from(tgt)),
                kind: EdgeKind::LeadsTo,
                polarity: EdgePolarity::Forward,
                label: Some(LABEL),
                weight: 1.0,
                home: EdgeHome::Context(home),
                emitter: EntityId::from(emitter),
            },
        )
        .await
        .and_then(|fired| fired.relationship())
    });
    await_blocked(&pool, holder, "%relationship_assert%", &asserter).await;

    act.commit().await.expect("the act commits");
    let fresh = asserter
        .await
        .expect("the asserter's task")
        .expect("the assertion that waited is admitted when it resumes");
    assert_ne!(fresh, x.e, "the assertion inserted another edge");

    // The erased edge stays erased: folded once, by the unit, under one record.
    assert_sentinelled(&pool, x.e).await;
    assert_eq!(folds_of(&pool, x.e).await.len(), 1, "the unit's fold alone");
    let records = records_of(&pool, x.e).await;
    assert_eq!(records.len(), 1, "one edge_erased record");

    // The new edge: live, at the same home, with the asserter's label, under its own event.
    let (folded, label, home_table, home_id): (bool, Option<String>, String, Uuid) =
        sqlx::query_as(
            "SELECT is_folded, label, home_anchor_table, home_anchor_id FROM kb_edges \
              WHERE id = $1",
        )
        .bind(fresh.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!folded, "the new edge is live");
    assert_eq!(label.as_deref(), Some(LABEL), "with the asserter's label");
    assert_eq!(
        (home_table.as_str(), home_id),
        ("kb_contexts", x.w.home.uuid()),
        "at the erased edge's home"
    );
    let fresh_asserted = asserting_event(&pool, fresh).await;
    assert_ne!(fresh_asserted, x.asserted, "under its own asserting event");
    let named: Option<String> =
        sqlx::query_scalar("SELECT payload ->> 'edge_id' FROM kb_events WHERE id = $1")
            .bind(fresh_asserted)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        named,
        Some(fresh.uuid().to_string()),
        "the new event names the new edge"
    );

    // Nothing joins the erased edge's erasure.
    let erased: bool = sqlx::query_scalar("SELECT _edge_is_erased($1)")
        .bind(fresh.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!erased, "the new edge is not erased");
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_event_field_redactions WHERE event_id = $1")
            .bind(fresh_asserted)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "no redaction row names the new event");
    assert_eq!(
        records[0]["redacted_fields"], out["redacted_fields"],
        "the record names what the unit rewrote, nothing after it"
    );

    // The ledger quotes the label once, in the new event, and nothing else of the erased edge.
    let quoting: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM kb_events \
          WHERE strpos(payload::text, $1) > 0 OR strpos(coalesce(metadata::text, ''), $1) > 0",
    )
    .bind(LABEL)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(quoting, vec![fresh_asserted], "the label is the asserter's");
    for needle in [REASONING, "confided"] {
        let hits: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_events \
              WHERE strpos(payload::text, $1) > 0 OR strpos(coalesce(metadata::text, ''), $1) > 0",
        )
        .bind(needle)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(hits, 0, "the ledger quotes {needle:?}");
    }
}

/// An edge between two of the world's resources with no label, asserted with no authorship: its
/// trail holds nothing to redact, so the unit projects no `edge` row for it.
async fn bare_edge(pool: &PgPool, w: &World, src: ResourceId, tgt: ResourceId) -> EdgeId {
    let mut conn = pool.acquire().await.expect("acquire");
    fire(
        &mut conn,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(src),
            tgt: AnchorRef::resource(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: None,
            weight: 1.0,
            home: EdgeHome::Context(w.home),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("assert the bare edge")
    .relationship()
    .expect("an edge id")
}

/// The unit on `e` in one transaction ([`erase_edge`]), then tombstoning the estate owner when
/// `tombstoned`, then the commit. The call itself must succeed; returns its result and the commit's
/// refusal text, if any.
async fn unit_then_commit(
    pool: &PgPool,
    x: &Estate,
    e: EdgeId,
    tombstoned: bool,
) -> (Value, Option<String>) {
    erase_edge(pool, e, x.w.profile, &x.op, Uuid::now_v7(), tombstoned)
        .await
        .expect("the call itself succeeds: the owner is checked at commit")
}

/// The edge is unchanged and no record names it.
async fn assert_untouched(pool: &PgPool, e: EdgeId, label: Option<&str>) {
    let (folded, held): (bool, Option<String>) =
        sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id = $1")
            .bind(e.uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(!folded, "the edge stays live");
    assert_eq!(held.as_deref(), label, "and keeps its label");
    assert!(
        records_of(pool, e).await.is_empty(),
        "no edge_erased record"
    );
    assert!(folds_of(pool, e).await.is_empty(), "no fold");
}

/// The unit is called with the estate's owner live, as no person erasure would call it. Each call
/// succeeds; the commit is refused by `edge_erasure_requires_an_erased_owner` and nothing persists.
/// The labelled edge's record projects `edge` rows; the bare edge's projects none, so only the
/// kb_events trigger reaches it. The twin tombstones the owner in the same transaction and commits.
///
/// FAILS IF the unit commits outside a person erasure: without
/// `trg_edge_erased_requires_an_erased_owner` the bare edge's act commits, and without both
/// triggers the labelled edge's does too; or if the check runs in the call rather than at commit;
/// or if the twin's tombstone does not admit the act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unit_outside_a_person_erasure_is_refused_at_commit(pool: PgPool) {
    let x = estate(&pool).await;
    let a = resource(&pool, &x.w, "delta").await;
    let b = resource(&pool, &x.w, "epsilon").await;
    let bare = bare_edge(&pool, &x.w, a, b).await;
    let refused = format!(
        "edge erasure: the estate's owner {} is not erased",
        x.w.profile
    );

    let before = event_count(&pool).await;
    let (out, err) = unit_then_commit(&pool, &x, x.e, false).await;
    assert_ne!(
        out["redacted_fields"],
        json!([]),
        "the labelled edge has rows"
    );
    let err = err.expect("the labelled edge's act is refused at commit");
    assert!(err.contains(&refused), "refused for its owner; got {err}");

    let (out, err) = unit_then_commit(&pool, &x, bare, false).await;
    assert_eq!(
        out["redacted_fields"],
        json!([]),
        "the bare edge has nothing to redact, so no `edge` row is projected"
    );
    let err = err.expect("the bare edge's act is refused at commit");
    assert!(err.contains(&refused), "refused for its owner; got {err}");

    assert_eq!(event_count(&pool).await, before, "nothing appended");
    assert_untouched(&pool, x.e, Some(LABEL)).await;
    assert_untouched(&pool, bare, None).await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_event_field_redactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "no redaction row persists");

    // The twin: the same act with the owner tombstoned before commit.
    let (_, err) = unit_then_commit(&pool, &x, bare, true).await;
    assert_eq!(
        err, None,
        "the bare edge's act commits once the owner is tombstoned"
    );
    let (_, err) = unit_then_commit(&pool, &x, x.e, true).await;
    assert_eq!(
        err, None,
        "the labelled edge's act commits once the owner is tombstoned"
    );
    assert_sentinelled(&pool, x.e).await;
    assert_eq!(
        records_of(&pool, bare).await.len(),
        1,
        "the bare edge's record"
    );
}
