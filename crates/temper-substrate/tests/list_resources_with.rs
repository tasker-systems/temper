#![cfg(feature = "test-db")]
//! **`list-resources-with` against a real database** (migration `20261128000010`).
//!
//! The list core is the selection core's set, narrowed by a bound, with each row's `created` as
//! `created_epoch`; the compiler orders, pages and counts it. This file holds the claims only a
//! database can check:
//!
//! - **the same mechanics** — unpaged and unbounded, the list core's set IS the selection core's
//!   set, over a corpus of predicates that admits and denies;
//! - **the order** — `created DESC, id ASC` by default and `created ASC, id ASC` under `asc`, with
//!   rows that share one `created`, so the tie is real;
//! - **the page** — walking by `offset` concatenates to the whole ordered set, and the tally's
//!   `total` is the unpaged count on every page, `0` included;
//! - **the bound sits beneath the page** — a bound id at position 60 of the unbounded order is still
//!   returned under `limit 50`.
//!
//! Every resource is created through the real write path with its embedding deferred, so no model
//! is needed.

mod common;

use std::collections::BTreeSet;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::types::query::filter::{OrdOp, PropertyOp, PropertyPredicate, ResourceFilter};
use temper_core::types::query::{
    validate, ActInvocation, ActName, BoundTerm, Composition, IdKind, IdSet, OutcomeDeclaration,
    ReturnSpec, StageInput, StageName, StageNode, StageRelation,
};
use temper_core::types::resource::SortOrder;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::readback::query_exec::{execute, QueryRows};
use temper_substrate::readback::query_plan::compile;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

// ── Harness ─────────────────────────────────────────────────────────────────────────────────────

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

struct World {
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
}

async fn world(pool: &PgPool) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = AnchorRef::context(ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), "listed", "listed")
            .await
            .unwrap(),
    ));
    World {
        owner,
        emitter,
        home,
    }
}

/// A resource homed in the world's home carrying `properties`, its embedding deferred.
async fn resource(pool: &PgPool, w: &World, title: &str, properties: &[(String, Value)]) -> Uuid {
    let origin = format!("test://list-resources-with/{title}");
    writes::create_resource_deferred_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &origin,
            body: "fixture body",
            doc_type: "task",
            home: w.home,
            owner: w.owner,
            originator: w.owner,
            emitter: w.emitter,
            properties,
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
    .uuid()
}

/// `n` resources carrying `state: ready`, returned in creation order.
async fn ready_rows(pool: &PgPool, w: &World, n: usize) -> Vec<Uuid> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(
            resource(
                pool,
                w,
                &format!("row-{i}"),
                &[("state".to_string(), json!("ready"))],
            )
            .await,
        );
    }
    out
}

/// Give `ids` one shared `created`, as rows written in one transaction have (`occurred_at` is the
/// transaction's `now()`). The projector writes `created` once and nothing updates it, so this
/// fixture write stands in for the one-transaction case rather than for anything production does.
async fn share_one_created(pool: &PgPool, ids: &[Uuid]) {
    sqlx::query(
        "UPDATE kb_resources SET created = (SELECT max(created) FROM kb_resources \
         WHERE id = ANY($1)) WHERE id = ANY($1)",
    )
    .bind(ids.to_vec())
    .execute(pool)
    .await
    .unwrap();
}

/// The ids the database orders the list by, in the declared order: `created` in `order`, then
/// `id ASC` — computed from the table, not from the statement under test.
async fn expected_order(pool: &PgPool, ids: &[Uuid], order: SortOrder) -> Vec<Uuid> {
    let mut rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT id, created FROM kb_resources WHERE id = ANY($1)")
            .bind(ids.to_vec())
            .fetch_all(pool)
            .await
            .unwrap();
    rows.sort_by(|a, b| match order {
        SortOrder::Desc => b.1.cmp(&a.1).then(a.0.cmp(&b.0)),
        SortOrder::Asc => a.1.cmp(&b.1).then(a.0.cmp(&b.0)),
    });
    rows.into_iter().map(|(id, _)| id).collect()
}

fn ready_filter() -> ResourceFilter {
    ResourceFilter {
        properties: vec![PropertyPredicate {
            key: "state".to_string(),
            op: PropertyOp::Contains {
                values: vec![json!("ready")],
            },
        }],
        ..Default::default()
    }
}

/// One returned `list-resources-with` stage named `rows`.
fn list_plan(
    inputs: Vec<StageInput>,
    filter: Option<ResourceFilter>,
    terms: &[(BoundTerm, i64)],
    order: Option<SortOrder>,
) -> Composition {
    let rows = StageName::parse("rows").unwrap();
    Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: rows.clone(),
                with: vec![],
            }],
        },
        stages: vec![StageNode::Act(ActInvocation {
            name: rows,
            act: ActName::ListResourcesWith,
            intention: None,
            inputs,
            terms: terms.iter().cloned().collect(),
            order,
            resource_filter: filter,
            edge_filter: None,
            properties: vec![],
            artifact_filter: vec![],
        })],
    }
}

fn anchored(w: &World) -> Vec<StageInput> {
    vec![StageInput::Caller {
        relation: StageRelation::Bound,
        ids: IdSet {
            kind: IdKind::Context,
            provenance: None,
            ids: vec![w.home.id],
        },
    }]
}

async fn run(pool: &PgPool, principal: ProfileId, c: &Composition) -> QueryRows {
    let v = validate(
        c,
        &mut temper_core::types::artifact_ask::ShapeContext::empty().session(),
    )
    .expect("the plan is valid");
    let q = compile(&v, principal).expect("the plan compiles");
    execute(pool, &q).await.expect("the statement runs")
}

/// The returned stage's ids in the order the assembler reads them (`hits_for`: quantity
/// descending, stable), and its tally's `produced` and `total`.
fn page_of(rows: &QueryRows) -> (Vec<Uuid>, i64, Option<i64>) {
    let ids = rows.hits_for("rows").into_iter().map(|h| h.id).collect();
    let t = rows
        .tallies
        .iter()
        .find(|t| t.stage == "rows")
        .expect("the stage is tallied");
    (ids, t.produced, t.total)
}

// ── The same mechanics ──────────────────────────────────────────────────────────────────────────

/// **Unpaged and unbounded, the list core's set is the selection core's set**, for the same
/// arguments, over a corpus of property predicates that both admits and denies.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_list_core_selects_exactly_what_the_selection_core_selects(pool: PgPool) {
    let w = world(&pool).await;
    let corpus: Vec<(&str, Vec<(String, Value)>)> = vec![
        (
            "a",
            vec![("state".into(), json!("ready")), ("rank".into(), json!(3))],
        ),
        (
            "b",
            vec![
                ("state".into(), json!("claimed")),
                ("claim_id".into(), json!("x")),
            ],
        ),
        (
            "c",
            vec![
                ("state".into(), json!(["ready", "hot"])),
                ("rank".into(), json!(9)),
            ],
        ),
        ("d", vec![("rank".into(), json!(1))]),
        ("e", vec![]),
    ];
    let mut all = Vec::new();
    for (title, props) in &corpus {
        all.push(resource(&pool, &w, title, props).await);
    }
    let predicates: Vec<Value> = vec![
        json!([{"key": "state", "op": {"op": "has_key"}}]),
        json!([{"key": "claim_id", "op": {"op": "has_key"}}]),
        json!([{"key": "state", "op": {"op": "contains", "values": ["ready"]}}]),
        serde_json::to_value(vec![PropertyPredicate {
            key: "rank".into(),
            op: PropertyOp::Compare {
                direction: OrdOp::Gt,
                value: json!(2),
            },
        }])
        .unwrap(),
        json!([]),
    ];
    let mut admitted_any = false;
    let mut denied_any = false;
    for p in predicates.iter().map(Some).chain([None]) {
        let select = |core: &str, extra: &str| {
            format!(
                "SELECT resource_id FROM {core}($1::uuid[], NULL::text[], NULL::text[], \
                 NULL::jsonb, NULL::text, NULL::text, NULL::uuid, NULL::text, NULL::text, \
                 NULL::jsonb, 'kb_contexts'::varchar, $2::uuid, $3::uuid, $4::jsonb{extra})"
            )
        };
        let read = |sql: String| {
            let pool = pool.clone();
            let all = all.clone();
            let p = p.cloned();
            let home = w.home.id;
            let owner = w.owner.uuid();
            async move {
                sqlx::query_scalar::<_, Uuid>(&sql)
                    .bind(all)
                    .bind(home)
                    .bind(owner)
                    .bind(p)
                    .fetch_all(&pool)
                    .await
                    .unwrap()
                    .into_iter()
                    .collect::<BTreeSet<Uuid>>()
            }
        };
        let selected = read(select("__temper_ungated_find_resources_with", "")).await;
        let listed = read(select(
            "__temper_ungated_list_resources_with",
            ", NULL::uuid[]",
        ))
        .await;
        assert_eq!(
            listed, selected,
            "the list and the selection disagree on {p:?}"
        );
        admitted_any |= !selected.is_empty();
        denied_any |= selected.len() < all.len();
    }
    assert!(
        admitted_any && denied_any,
        "the corpus must both admit and deny"
    );
}

// ── The order ───────────────────────────────────────────────────────────────────────────────────

/// **`created DESC, id ASC` by default, and `created ASC, id ASC` under `asc`**, with three rows
/// sharing one `created`, so the tiebreak is exercised rather than assumed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_list_orders_by_created_in_its_direction_and_breaks_ties_by_id(pool: PgPool) {
    let w = world(&pool).await;
    let ids = ready_rows(&pool, &w, 6).await;
    share_one_created(&pool, &ids[1..4]).await;

    for order in [None, Some(SortOrder::Desc), Some(SortOrder::Asc)] {
        let rows = run(
            &pool,
            w.owner,
            &list_plan(anchored(&w), Some(ready_filter()), &[], order),
        )
        .await;
        // `hits_for` is descending by quantity, stable; the assembler restores `asc`. Here the
        // statement's own arrival order is read, which is what the assembler's sorts preserve.
        let arrived: Vec<Uuid> = rows
            .hits
            .iter()
            .filter(|h| h.stage == "rows")
            .map(|h| h.id)
            .collect();
        let expected = expected_order(&pool, &ids, order.unwrap_or_default()).await;
        assert_eq!(arrived, expected, "{order:?}: the statement's order");
        if order != Some(SortOrder::Asc) {
            let (sorted, _, _) = page_of(&rows);
            assert_eq!(
                sorted, expected,
                "the stable descending re-sort keeps the id-ascending tie"
            );
        }
    }
}

// ── The page and its count ──────────────────────────────────────────────────────────────────────

/// **Walking by `offset` concatenates to the whole ordered set, and `total` is the unpaged count on
/// every page** — mid-walk, at the last short page, past the end, at `limit 0`, and `0` when
/// nothing matches.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_offset_walk_concatenates_and_every_page_carries_the_total(pool: PgPool) {
    let w = world(&pool).await;
    let ids = ready_rows(&pool, &w, 8).await;
    // One that does not match, so the count is the filter's and not the home's.
    resource(&pool, &w, "other", &[("state".to_string(), json!("done"))]).await;
    let expected = expected_order(&pool, &ids, SortOrder::Desc).await;

    let mut walked = Vec::new();
    for offset in [0, 3, 6] {
        let rows = run(
            &pool,
            w.owner,
            &list_plan(
                anchored(&w),
                Some(ready_filter()),
                &[(BoundTerm::Limit, 3), (BoundTerm::Offset, offset)],
                None,
            ),
        )
        .await;
        let (page, produced, total) = page_of(&rows);
        assert_eq!(total, Some(8), "offset {offset}: the unpaged count");
        assert_eq!(produced, page.len() as i64);
        walked.extend(page);
    }
    assert_eq!(walked, expected, "the pages concatenate to the ordered set");

    for (terms, produced) in [
        (vec![(BoundTerm::Limit, 3), (BoundTerm::Offset, 40)], 0),
        (vec![(BoundTerm::Limit, 0)], 0),
    ] {
        let rows = run(
            &pool,
            w.owner,
            &list_plan(anchored(&w), Some(ready_filter()), &terms, None),
        )
        .await;
        assert_eq!(page_of(&rows).1, produced, "{terms:?}");
        assert_eq!(
            page_of(&rows).2,
            Some(8),
            "{terms:?}: the count survives an empty page"
        );
    }

    let nothing = ResourceFilter {
        properties: vec![PropertyPredicate {
            key: "no_such_key".to_string(),
            op: PropertyOp::HasKey,
        }],
        ..Default::default()
    };
    let rows = run(
        &pool,
        w.owner,
        &list_plan(anchored(&w), Some(nothing), &[], None),
    )
    .await;
    assert_eq!(
        page_of(&rows),
        (vec![], 0, Some(0)),
        "zero matches count zero"
    );
}

/// **The bound applies beneath the page.** A bound naming the row at position 60 of the unbounded
/// order, beside 59 that precede it, still returns it under `limit 50` — the 50 come from the
/// bounded set. Moving the bound above the LIMIT would page the unbounded order first and lose it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_bound_applies_beneath_the_page(pool: PgPool) {
    let w = world(&pool).await;
    let ids = ready_rows(&pool, &w, 61).await;
    let order = expected_order(&pool, &ids, SortOrder::Desc).await;
    let sixtieth = order[59];
    let bound = vec![StageInput::Caller {
        relation: StageRelation::Bound,
        ids: IdSet {
            kind: IdKind::Resource,
            provenance: None,
            ids: vec![sixtieth, order[0]],
        },
    }];
    let rows = run(
        &pool,
        w.owner,
        &list_plan(bound, Some(ready_filter()), &[(BoundTerm::Limit, 50)], None),
    )
    .await;
    let (page, _, total) = page_of(&rows);
    assert_eq!(page, vec![order[0], sixtieth], "both bound rows, in order");
    assert_eq!(total, Some(2), "the count is the bounded match set's");

    // And unbounded, `limit 50` really does stop short of position 60.
    let rows = run(
        &pool,
        w.owner,
        &list_plan(
            anchored(&w),
            Some(ready_filter()),
            &[(BoundTerm::Limit, 50)],
            None,
        ),
    )
    .await;
    let (page, _, total) = page_of(&rows);
    assert_eq!(page.len(), 50);
    assert!(!page.contains(&sixtieth));
    assert_eq!(total, Some(61));
}

// ── Time ranges (`20261203000040`) ──────────────────────────────────────────────────────────────

/// Midnight UTC on day `d` of November 2026.
fn day(d: u32) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(2026, 11, d, 0, 0, 0)
        .single()
        .expect("a real day")
}

/// Pin `id`'s two clocks. The projector writes them; this fixture write stands in for resources
/// written on different days.
async fn set_clocks(
    pool: &PgPool,
    id: Uuid,
    created: chrono::DateTime<chrono::Utc>,
    updated: chrono::DateTime<chrono::Utc>,
) {
    sqlx::query("UPDATE kb_resources SET created = $2, updated = $3 WHERE id = $1")
        .bind(id)
        .bind(created)
        .bind(updated)
        .execute(pool)
        .await
        .unwrap();
}

fn ranged(
    created: Option<temper_core::types::query::TimeRange>,
    updated: Option<temper_core::types::query::TimeRange>,
) -> ResourceFilter {
    ResourceFilter {
        created,
        updated,
        ..ready_filter()
    }
}

fn range(
    from: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
) -> temper_core::types::query::TimeRange {
    temper_core::types::query::TimeRange { from, until }
}

/// **A `created` or `updated` range narrows at either end or both, `from` inclusive and `until`
/// exclusive, each over its own column** — on `list-resources-with` through the compiler, and on
/// the `find-resources-with` selection through its gated wrapper.
///
/// Five rows, `created` on days 1–5 and `updated` on days 5–1, so the two columns order the rows
/// oppositely and a range applied to the wrong column selects a different set. Each boundary row
/// sits exactly on an end, so an inclusive `until` or an exclusive `from` moves the answer.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_time_range_narrows_at_either_end_or_both_over_its_own_column(pool: PgPool) {
    let w = world(&pool).await;
    let ids = ready_rows(&pool, &w, 5).await;
    for (i, id) in ids.iter().enumerate() {
        let i = i as u32;
        set_clocks(&pool, *id, day(1 + i), day(5 - i)).await;
    }
    // Row k (0-based) was created on day k+1 and updated on day 5-k.
    let rows_at = |ks: &[usize]| ks.iter().map(|&k| ids[k]).collect::<BTreeSet<_>>();

    let cases = [
        (
            "created, from only",
            ranged(Some(range(Some(day(2)), None)), None),
            rows_at(&[1, 2, 3, 4]),
        ),
        (
            "created, until only",
            ranged(Some(range(None, Some(day(4)))), None),
            rows_at(&[0, 1, 2]),
        ),
        (
            "created, both",
            ranged(Some(range(Some(day(2)), Some(day(4)))), None),
            rows_at(&[1, 2]),
        ),
        (
            "updated, from only",
            ranged(None, Some(range(Some(day(4)), None))),
            rows_at(&[0, 1]),
        ),
        (
            "updated, until only",
            ranged(None, Some(range(None, Some(day(2))))),
            rows_at(&[4]),
        ),
        (
            "updated, both",
            ranged(None, Some(range(Some(day(2)), Some(day(4))))),
            rows_at(&[2, 3]),
        ),
        (
            "both ranges",
            ranged(
                Some(range(Some(day(2)), None)),
                Some(range(Some(day(3)), None)),
            ),
            rows_at(&[1, 2]),
        ),
        (
            "an empty range",
            ranged(Some(range(None, None)), None),
            rows_at(&[0, 1, 2, 3, 4]),
        ),
    ];
    for (case, filter, expected) in cases {
        let rows = run(
            &pool,
            w.owner,
            &list_plan(anchored(&w), Some(filter.clone()), &[], None),
        )
        .await;
        let (page, _, total) = page_of(&rows);
        assert_eq!(
            page.iter().copied().collect::<BTreeSet<_>>(),
            expected,
            "list-resources-with, {case}"
        );
        assert_eq!(total, Some(expected.len() as i64), "{case}: the total");

        // The selection's own door, over the same two ranges.
        let as_range = |r: &Option<temper_core::types::query::TimeRange>| {
            r.as_ref().map(|r| (r.from, r.until))
        };
        let (c, u) = (as_range(&filter.created), as_range(&filter.updated));
        let selected: BTreeSet<Uuid> = sqlx::query_scalar(
            "SELECT resource_id FROM query_find_resources_with($1, NULL::text[], NULL::text[], \
             NULL::jsonb, NULL::text, NULL::text, NULL::uuid, NULL::text, NULL::text, \
             NULL::jsonb, 'kb_contexts'::varchar, $2::uuid, NULL::jsonb, \
             CASE WHEN $3 THEN tstzrange($4, $5, '[)') END, \
             CASE WHEN $6 THEN tstzrange($7, $8, '[)') END)",
        )
        .bind(w.owner.uuid())
        .bind(w.home.id)
        .bind(c.is_some())
        .bind(c.and_then(|c| c.0))
        .bind(c.and_then(|c| c.1))
        .bind(u.is_some())
        .bind(u.and_then(|u| u.0))
        .bind(u.and_then(|u| u.1))
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .collect();
        assert_eq!(selected, expected, "find-resources-with, {case}");
    }
}

/// **A range is applied in the store, beneath the page**: the stage's `limit` still cuts the page,
/// and `total` counts the narrowed set, not the anchor's. Eight rows on days 1–8, a range admitting
/// days 3–7, `limit 2`: the page is the two newest of the five admitted, never of the eight.
/// Filtering the page after the read would answer fewer than two rows, or the wrong two, and a
/// total of eight.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_time_range_narrows_beneath_the_page_and_the_limit_still_holds(pool: PgPool) {
    let w = world(&pool).await;
    let ids = ready_rows(&pool, &w, 8).await;
    for (i, id) in ids.iter().enumerate() {
        set_clocks(&pool, *id, day(1 + i as u32), day(1 + i as u32)).await;
    }
    let filter = ranged(Some(range(Some(day(3)), Some(day(8)))), None);
    let rows = run(
        &pool,
        w.owner,
        &list_plan(anchored(&w), Some(filter), &[(BoundTerm::Limit, 2)], None),
    )
    .await;
    let (page, produced, total) = page_of(&rows);
    assert_eq!(
        page,
        vec![ids[6], ids[5]],
        "the two newest admitted rows, newest first"
    );
    assert_eq!(produced, 2, "the limit holds");
    assert_eq!(total, Some(5), "the total counts the narrowed set");
}
