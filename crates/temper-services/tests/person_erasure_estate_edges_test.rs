#![cfg(feature = "test-db")]
//! Witnesses for the person act's edge step (migration 20261202000040, `principal_erasure_execute`):
//! after its resource erasures, the act runs `edge_erasure_execute` for every edge homed in an
//! estate context whose text is not yet erased (`_edge_erasure_complete`), live or folded before
//! the act, whatever the edge points at.
//!
//! Each world is built through the real write paths (`edge_erasure`'s builders) and erased through
//! `erasure_service::execute_erasure`, as `person_erasure_estate_test.rs` does. Only the historical
//! re-assertion is appended by SQL, as every version before 20261117000030 recorded one, and so is
//! a husk as cut 1 left it. Without the edge step the act reaches an edge only through an estate
//! resource at either end.

mod edge_erasure;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::EstateDisposition;
use temper_core::types::ids::ProfileId;
use temper_core::types::property_owner::PropertyOwner;
use temper_services::error::ApiError;
use temper_services::services::erasure_service::{
    execute_erasure, survey_erasure, ErasureCompletion,
};
use temper_services::services::resource_erasure_service::{
    execute_resource_erasure, ResourceErasureOutcome, ResourceErasureRequest,
};
use temper_services::test_support;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::replay;
use temper_substrate::writes;
use temper_workflow::operations::Surface;

use edge_erasure::{
    asserting_event, authored, edge, edge_in, facet, fold, historical_reassertion, reset_namespace,
    resource, resource_in, team_context, world, World,
};

const LABEL: &str = "jane roe told us";
const REASONING: &str = "jane roe said so on the call";
const SCALAR: &str = "jane roe diagnosis";
const UNSET: &str = "jane roe remark, retracted";
/// Text the erased edge alone carries: its label, its authorship reasoning, a facet mark's value,
/// a scalar property's value and a retracted property's value.
const NEEDLES: [&str; 5] = [LABEL, REASONING, "confided", SCALAR, UNSET];
/// `_principal_erasure_edges_outcome(1)`.
const ONE_EDGE: &str = "1 edge(s) homed in the estate erased: folded, labels cleared, edge-owned \
                        properties and ledger text sentinel-scrubbed";

async fn operator(pool: &PgPool) -> temper_services::auth::SystemAdmin {
    let op = world(pool).await;
    test_support::grant_governance(pool, op.profile).await;
    test_support::system_admin_proof_for(pool, op.profile).await
}

/// The person act on `subject` through the service, under a fresh operator and request.
async fn erase(pool: &PgPool, subject: Uuid) -> ErasureCompletion {
    let admin = operator(pool).await;
    execute_erasure(
        pool,
        &admin,
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
        &Default::default(),
    )
    .await
    .expect("the person act completes")
}

/// Ledger rows (payload or metadata) whose text contains `needle`.
async fn ledger_quoting(pool: &PgPool, needle: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events \
          WHERE strpos(payload::text, $1) > 0 OR strpos(coalesce(metadata::text, ''), $1) > 0",
    )
    .bind(needle)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The `relationship_folded` events naming the edge: (payload, correlation id), in order.
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

/// The `edge_erased` records naming the edge: (event id, payload), in order.
async fn records_of(pool: &PgPool, e: EdgeId) -> Vec<(Uuid, Value)> {
    sqlx::query_as(
        "SELECT ev.id, ev.payload FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'edge_erased' AND (ev.payload ->> 'subject_id')::uuid = $1 \
          ORDER BY ev.id",
    )
    .bind(e.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The edge's (is_folded, label).
async fn edge_state(pool: &PgPool, e: EdgeId) -> (bool, Option<String>) {
    sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id = $1")
        .bind(e.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The edge is folded with no label, and every property row it owns is folded at the sentinels.
async fn assert_sentinelled(pool: &PgPool, e: EdgeId) {
    let (folded, label) = edge_state(pool, e).await;
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

/// One labelled edge, homed in `home`, between `src` and `tgt`, with authorship reasoning, a scalar
/// property, a two-mark facet, a property set then retracted, and a historical re-assertion.
async fn rich_edge(
    pool: &PgPool,
    w: &World,
    home: ContextId,
    src: ResourceId,
    tgt: ResourceId,
) -> EdgeId {
    let e = edge_in(pool, w, home, src, tgt, LABEL, authored(REASONING)).await;
    writes::assert_keyed_property_with(
        pool,
        PropertyOwner::Edge { id: e },
        "diagnosis",
        &json!(SCALAR),
        1.0,
        EntityId::from(w.emitter),
        EventContext::default(),
    )
    .await
    .expect("a scalar property on the edge");
    facet(
        pool,
        w,
        e,
        json!({ "node_label": "jane roe", "status": "confided" }),
    )
    .await;
    let unset = writes::assert_keyed_property_with(
        pool,
        PropertyOwner::Edge { id: e },
        "remark",
        &json!(UNSET),
        1.0,
        EntityId::from(w.emitter),
        EventContext::default(),
    )
    .await
    .expect("a property to retract");
    writes::retract_property_with(
        pool,
        e,
        unset,
        EntityId::from(w.emitter),
        EventContext::default(),
    )
    .await
    .expect("retract the property");
    historical_reassertion(pool, w, e).await;
    for needle in NEEDLES {
        assert!(
            ledger_quoting(pool, needle).await > 0,
            "setup: the ledger holds {needle:?}"
        );
    }
    e
}

/// The `erasure` references of the act's record.
async fn erasure_references(pool: &PgPool, completion: &ErasureCompletion) -> Vec<Uuid> {
    let refs: Value = sqlx::query_scalar("SELECT \"references\" FROM kb_events WHERE id = $1")
        .bind(completion.event_id)
        .fetch_one(pool)
        .await
        .unwrap();
    refs.as_array()
        .expect("references")
        .iter()
        .filter(|r| r["rel"] == "erasure")
        .map(|r| r["target"]["id"].as_str().unwrap().parse().unwrap())
        .collect()
}

/// W1. FAILS IF the act has no edge step, because the edge stays live and labelled: an edge homed
/// in the subject's `@me` context between two team resources has no estate endpoint, so no resource
/// erasure reaches it. Through the act it must end folded, its label NULL, its owned properties at
/// the sentinels, no ledger copy of its text, one `edge_erased` record (estate_of the subject,
/// folded true) that the act's record references, and the record's kb_edges target naming one edge.
/// The act committing at all shows the deferred owner check passed inside it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_homed_in_the_estate_between_team_resources_is_folded_and_erased(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let r = resource_in(&pool, &w, team, "alpha").await;
    let t = resource_in(&pool, &w, team, "beta").await;
    let e = rich_edge(&pool, &w, w.home, r, t).await;

    let completion = erase(&pool, w.profile).await;

    assert_sentinelled(&pool, e).await;
    for needle in NEEDLES {
        assert_eq!(
            ledger_quoting(&pool, needle).await,
            0,
            "no ledger row quotes {needle:?} after the act"
        );
    }
    let records = records_of(&pool, e).await;
    assert_eq!(records.len(), 1, "one edge_erased record, committed");
    let (record_id, record) = &records[0];
    assert_eq!(record["estate_of"], json!(w.profile));
    assert_eq!(
        record["folded"],
        json!(true),
        "the act folded the live edge"
    );
    let folds = folds_of(&pool, e).await;
    assert_eq!(folds.len(), 1, "one fold");
    assert_eq!(folds[0].0["reason"], json!("estate_erased"));
    let request: Option<Uuid> =
        sqlx::query_scalar("SELECT correlation_id FROM kb_events WHERE id = $1")
            .bind(completion.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(folds[0].1, request, "the fold is the act's");
    assert!(
        erasure_references(&pool, &completion)
            .await
            .contains(record_id),
        "the act's record references the edge_erased"
    );
    assert!(
        completion
            .targets
            .iter()
            .any(|x| x.target == "kb_edges" && x.outcome == ONE_EDGE),
        "the record counts the edge: {:?}",
        completion.targets
    );
    let tombstoned: bool =
        sqlx::query_scalar("SELECT tombstoned_at IS NOT NULL FROM kb_profiles WHERE id = $1")
            .bind(w.profile)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(tombstoned, "the act committed with its subject tombstoned");
    for (res, what) in [(r, "the source"), (t, "the target")] {
        let erased: bool =
            sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
                .bind(res.uuid())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!erased, "{what}, a team resource, is not erased");
    }
}

/// W2. FAILS IF the act has no edge step, because the edge stays live and labelled: a resource in
/// `@me` asserts an edge to a team resource, homed in `@me`, and is then moved to a team context
/// through the real rehome write. The edge keeps its home, so the act reaches it by that home and
/// erases it as in W1, and both resources stay live.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_moved_out_resource_leaves_its_estate_homed_edge_to_the_act(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let r = resource(&pool, &w, "moving page").await;
    let t = resource_in(&pool, &w, team, "team page").await;
    let e = rich_edge(&pool, &w, w.home, r, t).await;
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::ResourceRehome {
            resource: r,
            home: AnchorRef::context(team),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("move the resource out of the estate");
    drop(conn);
    let home: (String, Uuid) =
        sqlx::query_as("SELECT home_anchor_table, home_anchor_id FROM kb_edges WHERE id = $1")
            .bind(e.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        home,
        ("kb_contexts".to_owned(), w.home.uuid()),
        "setup: the move leaves the edge homed in the estate"
    );

    let completion = erase(&pool, w.profile).await;

    assert!(
        completion.resource_erasures.is_empty(),
        "no estate resource is left to erase"
    );
    assert_sentinelled(&pool, e).await;
    for needle in NEEDLES {
        assert_eq!(ledger_quoting(&pool, needle).await, 0, "{needle:?}");
    }
    assert_eq!(
        records_of(&pool, e).await.len(),
        1,
        "one edge_erased record"
    );
    for res in [r, t] {
        let erased: bool =
            sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
                .bind(res.uuid())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!erased, "resource {} is untouched", res.uuid());
    }
}

/// W3. FAILS IF the plan's trail leaves out the estate-homed edges' own trails, because R8 then
/// counts the edge's label (and its property values) as text the subject kept outside the estate:
/// the subject authored the edge, and its events are on no estate resource's trail. The plan's
/// trail now holds every estate-homed edge's own trail, so the survey names no `edge labels` or
/// `property values` entry, and it predicts the edge step's kb_edges target exactly as the act then
/// records it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn r8_counts_an_estate_homed_edge_label_inside_the_estate(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let r = resource_in(&pool, &w, team, "alpha").await;
    let t = resource_in(&pool, &w, team, "beta").await;
    rich_edge(&pool, &w, w.home, r, t).await;
    let admin = operator(&pool).await;

    let survey = survey_erasure(&pool, &admin, ProfileId::from(w.profile), None)
        .await
        .expect("survey");

    for class in ["edge labels", "property values"] {
        let target = format!("kb_events.payload ({class})");
        assert!(
            !survey.targets.iter().any(|x| x.target == target),
            "no {target} remainder for the estate-homed edge: {:?}",
            survey.targets
        );
    }
    let predicted: Vec<_> = survey
        .targets
        .iter()
        .filter(|x| x.target == "kb_edges")
        .collect();
    assert_eq!(
        predicted.len(),
        1,
        "one kb_edges target: {:?}",
        survey.targets
    );
    assert_eq!(predicted[0].outcome, ONE_EDGE);

    let completion = erase(&pool, w.profile).await;
    assert_eq!(
        survey.targets, completion.targets,
        "the survey predicted the record's targets"
    );
}

/// W4. A team-homed edge into the estate is resource erasure's, as before: folded once, reason
/// `resource_erased`, label NULL, properties at the sentinels, and no `edge_erased` record. Beside
/// it an estate-homed edge between team resources is erased by the edge step. FAILS IF the act has
/// no edge step, because that second edge stays live; FAILS IF the edge step selects by anything
/// but the estate home (an `edge_erased` record, or a second fold, for the team-homed edge).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_team_homed_edge_into_the_estate_is_handled_as_before(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let estate_page = resource(&pool, &w, "estate page").await;
    let t = resource_in(&pool, &w, team, "team page").await;
    let into = edge_in(&pool, &w, team, t, estate_page, LABEL, authored(REASONING)).await;
    facet(
        &pool,
        &w,
        into,
        json!({ "node_label": "jane roe", "status": "confided" }),
    )
    .await;
    let a = resource_in(&pool, &w, team, "alpha").await;
    let b = resource_in(&pool, &w, team, "beta").await;
    let homed = edge(
        &pool,
        &w,
        a,
        b,
        "an estate edge's label",
        authored("its reasoning"),
    )
    .await;
    facet(&pool, &w, homed, json!({ "status": "homed here" })).await;

    erase(&pool, w.profile).await;

    assert_sentinelled(&pool, into).await;
    let folds = folds_of(&pool, into).await;
    assert_eq!(folds.len(), 1, "one fold of the team-homed edge");
    assert_eq!(folds[0].0["reason"], json!("resource_erased"));
    assert!(
        records_of(&pool, into).await.is_empty(),
        "no edge_erased for the team-homed edge"
    );
    assert_sentinelled(&pool, homed).await;
    assert_eq!(
        records_of(&pool, homed).await.len(),
        1,
        "the estate-homed edge is erased by the edge step"
    );
}

/// W5. FAILS IF the act has no edge step, because it then appends no `estate_erased` fold to keep.
/// After the act erases an estate-homed edge R→T, a second live edge between the same pair is
/// asserted in a team context, and T is then erased by a standalone resource erasure. That later
/// act must leave the first edge's `estate_erased` fold as it is and not fold it again, and must
/// number the second edge's label after the first's sentinel: `erased-label-2`, never a second
/// `erased-label-1`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_later_erasure_of_an_endpoint_leaves_the_edge_act_record(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let r = resource_in(&pool, &w, team, "alpha").await;
    let t = resource_in(&pool, &w, team, "beta").await;
    let first = rich_edge(&pool, &w, w.home, r, t).await;
    erase(&pool, w.profile).await;
    let first_asserted = asserting_event(&pool, first).await;
    let first_label: Option<String> =
        sqlx::query_scalar("SELECT payload ->> 'label' FROM kb_events WHERE id = $1")
            .bind(first_asserted)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(first_label.as_deref(), Some("erased-label-1"));

    let member = world(&pool).await;
    let second = edge_in(
        &pool,
        &member,
        team,
        r,
        t,
        "a later label on the same pair",
        EventContext::default(),
    )
    .await;
    assert_ne!(second, first, "a new edge between the same pair");
    let admin = operator(&pool).await;
    let outcome = execute_resource_erasure(
        &pool,
        None,
        &admin,
        ResourceErasureRequest {
            resource: t,
            also_strike_blobs: &[],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("the resource act runs");
    assert!(matches!(outcome, ResourceErasureOutcome::Completed(_)));

    let folds = folds_of(&pool, first).await;
    assert_eq!(folds.len(), 1, "the erased edge is not folded again");
    assert_eq!(
        folds[0].0["reason"],
        json!("estate_erased"),
        "the edge act's fold reason is kept"
    );
    let second_label: Option<String> =
        sqlx::query_scalar("SELECT payload ->> 'label' FROM kb_events WHERE id = $1")
            .bind(asserting_event(&pool, second).await)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        second_label.as_deref(),
        Some("erased-label-2"),
        "the second edge's label gets a fresh number"
    );
    let first_again: Option<String> =
        sqlx::query_scalar("SELECT payload ->> 'label' FROM kb_events WHERE id = $1")
            .bind(first_asserted)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(first_again.as_deref(), Some("erased-label-1"));
    assert_eq!(edge_state(&pool, second).await, (true, None));
}

/// W6. FAILS IF the edge step selects only live edges, because an edge folded before the act then
/// keeps its label and its trail's text. Ruling 8 of decision
/// 01a1f6e8-81bd-758c-be87-a6824b66352a: the act erases it too, with no new fold, and its record
/// says folded false.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_estate_edge_folded_before_the_act_is_erased_without_a_new_fold(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let r = resource_in(&pool, &w, team, "alpha").await;
    let t = resource_in(&pool, &w, team, "beta").await;
    let e = rich_edge(&pool, &w, w.home, r, t).await;
    fold(&pool, &w, e).await;
    assert_eq!(edge_state(&pool, e).await, (true, Some(LABEL.to_owned())));

    let completion = erase(&pool, w.profile).await;

    assert_sentinelled(&pool, e).await;
    for needle in NEEDLES {
        assert_eq!(ledger_quoting(&pool, needle).await, 0, "{needle:?}");
    }
    let folds = folds_of(&pool, e).await;
    assert_eq!(folds.len(), 1, "no new fold: only the ordinary one");
    assert_ne!(
        folds[0].0["reason"],
        json!("estate_erased"),
        "the ordinary fold, not one the act appended"
    );
    let records = records_of(&pool, e).await;
    assert_eq!(records.len(), 1, "one edge_erased record");
    assert_eq!(
        records[0].1["folded"],
        json!(false),
        "the record folded nothing"
    );
    assert!(completion
        .targets
        .iter()
        .any(|x| x.target == "kb_edges" && x.outcome == ONE_EDGE));
}

/// W7. An estate-homed edge from an estate resource to a team resource is folded and erased by
/// that resource's erasure earlier in the same act. It is then complete (`_edge_erasure_complete`),
/// so the edge step skips it: the act completes, the edge has exactly one fold (reason
/// `resource_erased`) and no `edge_erased` record. Beside it an estate-homed edge with no estate
/// endpoint is erased by the step. FAILS IF the act has no edge step, because that second edge
/// stays live; FAILS IF the step selects the first edge too, because the unit then raises
/// `already erased` and the act aborts.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_a_resource_erasure_folds_in_the_same_act_is_not_erased_twice(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let estate_page = resource(&pool, &w, "estate page").await;
    let t = resource_in(&pool, &w, team, "team page").await;
    let reached = rich_edge(&pool, &w, w.home, estate_page, t).await;
    let a = resource_in(&pool, &w, team, "alpha").await;
    let b = resource_in(&pool, &w, team, "beta").await;
    let stranded = edge(
        &pool,
        &w,
        a,
        b,
        "a stranded label",
        authored("its reasoning"),
    )
    .await;
    facet(&pool, &w, stranded, json!({ "status": "stranded" })).await;

    let completion = erase(&pool, w.profile).await;

    assert_eq!(completion.resource_erasures.len(), 1, "the estate page");
    assert_sentinelled(&pool, reached).await;
    let folds = folds_of(&pool, reached).await;
    assert_eq!(folds.len(), 1, "exactly one fold");
    assert_eq!(folds[0].0["reason"], json!("resource_erased"));
    assert!(
        records_of(&pool, reached).await.is_empty(),
        "no edge_erased for the edge the resource erasure erased"
    );
    assert_sentinelled(&pool, stranded).await;
    assert_eq!(records_of(&pool, stranded).await.len(), 1);
    assert!(completion
        .targets
        .iter()
        .any(|x| x.target == "kb_edges" && x.outcome == ONE_EDGE));
}

/// The count of events of `kind` correlated with `request`.
async fn span_count(pool: &PgPool, kind: &str, request: Option<Uuid>) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = $1 AND ev.correlation_id = $2",
    )
    .bind(kind)
    .bind(request)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// W8. FAILS IF the act has no edge step, because its span then holds no `edge_erased` record. The
/// person act's own span holds a resource erasure (with the fold of its estate-homed edge), an edge
/// erasure of a live estate-homed edge and one of an edge folded before the act, and the record.
/// Replay must rebuild every projection table byte for byte; it FAILS too IF replay places an edge
/// body where the live act did not leave it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_of_the_person_act_with_estate_edges_is_byte_identical(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let estate_page = resource(&pool, &w, "estate page").await;
    let t = resource_in(&pool, &w, team, "team page").await;
    edge(
        &pool,
        &w,
        estate_page,
        t,
        "the estate page's own label",
        authored("the estate page's reasoning"),
    )
    .await;
    let a = resource_in(&pool, &w, team, "alpha").await;
    let b = resource_in(&pool, &w, team, "beta").await;
    rich_edge(&pool, &w, w.home, a, b).await;
    let c = resource_in(&pool, &w, team, "gamma").await;
    let d = resource_in(&pool, &w, team, "delta").await;
    let folded = edge(
        &pool,
        &w,
        c,
        d,
        "another free label",
        authored("folded first"),
    )
    .await;
    facet(&pool, &w, folded, json!({ "status": "folded first" })).await;
    fold(&pool, &w, folded).await;

    let completion = erase(&pool, w.profile).await;
    let request: Option<Uuid> =
        sqlx::query_scalar("SELECT correlation_id FROM kb_events WHERE id = $1")
            .bind(completion.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(span_count(&pool, "resource_erased", request).await, 1);
    assert_eq!(
        span_count(&pool, "edge_erased", request).await,
        2,
        "the live and the folded estate-homed edges"
    );
    assert_eq!(span_count(&pool, "principal_erased", request).await, 1);

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    reset_namespace(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    assert_eq!(before.len(), after.len(), "the same projection tables");
    for ((table_a, x), (table_b, y)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(x, y, "projection table {table_a} diverged under replay");
    }
}

/// A husk as cut 1 left it (`person_erasure_estate_test`'s `simulate_cut1_husk`): a
/// `resource_erased` naming no `redacted_fields` and no folded edge, and the projection-side body,
/// with the ledger's text untouched and its edges left live. After it commits,
/// `_erasure_legacy_backfill()` runs, as `20261201000030` runs it over a ledger erased before the
/// erasure record existed. The person act's plan then marks the husk `complete`.
async fn cut1_husk(pool: &PgPool, w: &World, resource: ResourceId) {
    let request_ref = Uuid::now_v7();
    let mut tx = pool.begin().await.unwrap();
    let plan: Value = sqlx::query_scalar("SELECT resource_erasure_survey_plan($1)")
        .bind(resource.uuid())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let erased: Uuid = sqlx::query_scalar(
        "SELECT _event_append('resource_erased', $2, NULL, NULL, jsonb_build_object( \
             'subject_table', 'kb_resources', 'subject_id', $1, 'actor', $3, \
             'folded_edges', '[]'::jsonb, 'targets', $4->'targets', 'remainder', $4->'remainder', \
             'ledger_remainder', ($4->'redacted_fields') || ($4->'ledger_remainder')), \
             p_correlation => $5)",
    )
    .bind(resource.uuid())
    .bind(w.emitter)
    .bind(w.profile)
    .bind(&plan)
    .bind(request_ref)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SELECT _resource_erasure_apply_redaction($1, $2)")
        .bind(resource.uuid())
        .bind(erased)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::query("SELECT _erasure_legacy_backfill()")
        .execute(pool)
        .await
        .unwrap();
}

/// The record's kb_edges outcomes, in order.
fn edge_outcomes(targets: &[temper_core::types::erasure::ErasureTargetOutcome]) -> Vec<String> {
    targets
        .iter()
        .filter(|x| x.target == "kb_edges")
        .map(|x| x.outcome.clone())
        .collect()
}

/// W9. The survey predicts the edge step from the state the resource erasures before it leave, so
/// its kb_edges target is the act's, for estate-homed edges touching a `skip` and a `complete`
/// husk. The `skip` husk, erased by a standalone resource erasure, left its edge complete: neither
/// the survey nor the act counts it. The `complete` husk, a cut-1 husk whose edge was never folded,
/// gets a completion pass, which folds nothing, so its live edge is the step's: the survey
/// predicts one edge and the act erases one. FAILS IF the survey excludes every edge touching a
/// `complete` resource: it predicts none, and the act's record names one.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_predicts_the_edge_step_over_skip_and_complete_husks(pool: PgPool) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let t = resource_in(&pool, &w, team, "team page").await;
    let skipped = resource(&pool, &w, "complete husk").await;
    let reached = edge(
        &pool,
        &w,
        skipped,
        t,
        "the complete husk's label",
        authored("its reasoning"),
    )
    .await;
    let completed = resource(&pool, &w, "cut-1 husk").await;
    let live = edge(&pool, &w, completed, t, LABEL, authored(REASONING)).await;
    let admin = operator(&pool).await;
    let outcome = execute_resource_erasure(
        &pool,
        None,
        &admin,
        ResourceErasureRequest {
            resource: skipped,
            also_strike_blobs: &[],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("the resource act runs");
    assert!(matches!(outcome, ResourceErasureOutcome::Completed(_)));
    cut1_husk(&pool, &w, completed).await;
    assert_eq!(
        edge_state(&pool, live).await,
        (false, None),
        "setup: the cut-1 husk's edge is live, its label nulled by the projection body"
    );

    let survey = survey_erasure(&pool, &admin, ProfileId::from(w.profile), None)
        .await
        .expect("survey");
    let disposition = |r: ResourceId| {
        survey
            .resources
            .iter()
            .find(|x| x.resource_id == r.uuid())
            .map(|x| x.disposition)
    };
    assert_eq!(disposition(skipped), Some(EstateDisposition::Skip));
    assert_eq!(disposition(completed), Some(EstateDisposition::Complete));
    let predicted = edge_outcomes(&survey.targets);
    assert_eq!(
        predicted,
        vec![ONE_EDGE.to_owned()],
        "the survey predicts the cut-1 husk's live edge: {:?}",
        survey.targets
    );

    let completion = erase(&pool, w.profile).await;

    assert_eq!(
        edge_outcomes(&completion.targets),
        predicted,
        "the act erased what the survey predicted"
    );
    assert_eq!(
        records_of(&pool, live).await.len(),
        1,
        "the cut-1 husk's live edge is erased by the edge step"
    );
    assert_eq!(edge_state(&pool, live).await, (true, None));
    assert!(
        records_of(&pool, reached).await.is_empty(),
        "the complete husk's edge was already complete"
    );
}

/// W10. A context that leaves the estate between the act's plan and its context lock is decided by
/// two estates, the residual 20261202000040's `principal_erasure_execute` documents at its context
/// lock (ruling 11 of decision 01a1f6e8-81bd-758c-be87-a6824b66352a, (a)). The act plans with the
/// personal team's context in the estate, then waits on the act queue of a resource homed there;
/// meanwhile the team stops being personal, so the context leaves the estate. Then:
///   * the resource loop re-checks a resource's home against the plan's estate, so the waited
///     resource is erased;
///   * `_principal_erasure_contexts_to_erase` reads the plan's estate, so the context gets one
///     `context_erased`;
///   * the edge step re-checks each edge's home against the live estate under its lock, so the
///     edge homed there is skipped, kept and counted in the record;
///   * `_principal_erasure_context_events` reads the live estate, so the context's earlier
///     `context_renamed` keeps its name and slug on the ledger.
///
/// FAILS IF the act decides any of the four by the other estate. The edge step trusting the plan
/// fails the act, since the unit raises `not homed in the estate`, which the service does not
/// retry. The route is a stand-in: no door clears `personal_of`. Through a door the context leaves
/// by `context_reassign`, which waits on the act only once the act holds the context's row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_context_leaving_the_estate_mid_act_is_decided_by_the_plan_and_the_live_estate(
    pool: PgPool,
) {
    let w = world(&pool).await;
    let team = team_context(&pool).await;
    let personal_team = test_support::profile::personal_team(&pool, w.profile).await;
    let journal = ContextId::from(
        test_support::context::insert_team(&pool, personal_team, "journal", "journal").await,
    );
    let waited = resource_in(&pool, &w, journal, "journal page").await;
    let a = resource_in(&pool, &w, team, "alpha").await;
    let b = resource_in(&pool, &w, team, "beta").await;
    let e = edge_in(&pool, &w, journal, a, b, LABEL, authored(REASONING)).await;
    writes::rename_context_with(
        &pool,
        journal,
        ("journal", "journal"),
        (DEPARTED_NAME, DEPARTED_SLUG),
        EntityId::from(w.emitter),
        EventContext::default(),
    )
    .await
    .expect("rename through the real path");
    let admin = operator(&pool).await;

    // The journal page's act queue, held: the person act plans, then waits for it.
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(_resource_act_queue_key($1))")
        .bind(waited.uuid())
        .execute(&mut *held)
        .await
        .unwrap();
    let pool2 = pool.clone();
    let subject = w.profile;
    let act = tokio::spawn(async move {
        execute_erasure(
            &pool2,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
            &Default::default(),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    sqlx::query("UPDATE kb_teams SET personal_of = NULL WHERE id = $1")
        .bind(personal_team)
        .execute(&pool)
        .await
        .unwrap();
    held.rollback().await.unwrap();
    let completion = act.await.unwrap().expect("the person act completes");

    assert!(
        completion.targets.iter().any(|x| x.target == "kb_edges"
            && x.outcome
                == "1 edge(s) homed in a context that left the estate after the plan; skipped"),
        "the skipped edge is counted: {:?}",
        completion.targets
    );
    assert!(
        records_of(&pool, e).await.is_empty(),
        "no edge_erased for the edge whose context left the estate"
    );
    assert_eq!(edge_state(&pool, e).await, (false, Some(LABEL.to_owned())));

    // The projection erasure follows the plan.
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1"
        )
        .bind(waited.uuid())
        .fetch_one(&pool)
        .await
        .unwrap(),
        "the departed context's resource is erased"
    );
    let closures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'context_erased' AND e.producing_anchor_table = 'kb_contexts' \
            AND e.producing_anchor_id = $1",
    )
    .bind(journal.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(closures, 1, "one context_erased for the departed context");

    // The ledger rewrite follows the live estate: the earlier rename keeps its text.
    let renamed: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT e.payload->>'to_name', e.payload->>'to_slug' \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'context_renamed' AND e.producing_anchor_table = 'kb_contexts' \
            AND e.producing_anchor_id = $1",
    )
    .bind(journal.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        renamed,
        (
            Some(DEPARTED_NAME.to_owned()),
            Some(DEPARTED_SLUG.to_owned())
        ),
        "the departed context's earlier name and slug are kept on the ledger"
    );
}

/// W10's rename target: text the ledger keeps for a context that left the estate mid-act.
const DEPARTED_NAME: &str = "Departed Journal";
const DEPARTED_SLUG: &str = "departed-journal";

/// The label every assertion of an endpoint pair carries, in assertion order: the payload label of
/// each `relationship_asserted` from `src` to `tgt`.
async fn pair_labels(pool: &PgPool, src: ResourceId, tgt: ResourceId) -> Vec<Option<String>> {
    sqlx::query_scalar(
        "SELECT ev.payload ->> 'label' FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'relationship_asserted' \
            AND ev.payload #>> '{source,id}' = $1::text AND ev.payload #>> '{target,id}' = $2::text \
          ORDER BY ev.id",
    )
    .bind(src.uuid())
    .bind(tgt.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// `_edge_erasure_complete(e)`.
async fn edge_complete(pool: &PgPool, e: EdgeId) -> bool {
    sqlx::query_scalar("SELECT _edge_erasure_complete($1)")
        .bind(e.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// W11. A team-homed edge asserts a label from S to T first; the subject's estate-homed edge then
/// asserts the same label on the same pair. The act erases the estate edge alone, its label to
/// `erased-label-1`, and that rewrite is final: right after the act the edge is complete, the survey
/// predicts no edge, and a second person act on the tombstoned subject runs clean.
///
/// FAILS IF the shared loop derives a path an `edge` row already names, or the label numbering
/// counts the pair's written sentinels as labels. The team edge's earlier label then ranks 1 and
/// the written `erased-label-1` ranks 2, so the derivation right after the act rewrites the erased
/// edge's label again: `_edge_erasure_complete` is false, the survey keeps predicting the edge, and
/// the second act re-projects (asserting event, `label`, `edge`), which collides on
/// kb_event_field_redactions_pkey and fails the act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_label_shared_with_an_edge_homed_outside_the_estate_keeps_its_sentinel_stable(
    pool: PgPool,
) {
    let w = world(&pool).await;
    let member = world(&pool).await;
    let team = team_context(&pool).await;
    let s = resource_in(&pool, &w, team, "alpha").await;
    let t = resource_in(&pool, &w, team, "beta").await;
    let outside = edge_in(&pool, &member, team, s, t, SHARED, EventContext::default()).await;
    let homed = edge_in(&pool, &w, w.home, s, t, SHARED, EventContext::default()).await;

    erase(&pool, w.profile).await;

    assert_eq!(
        pair_labels(&pool, s, t).await,
        vec![Some(SHARED.to_owned()), Some("erased-label-1".to_owned())],
        "the team edge keeps its label; the estate edge's is at its sentinel"
    );
    assert!(
        edge_complete(&pool, homed).await,
        "the erased edge is complete right after the act"
    );
    let admin = operator(&pool).await;
    let survey = survey_erasure(&pool, &admin, ProfileId::from(w.profile), None)
        .await
        .expect("survey");
    assert!(
        edge_outcomes(&survey.targets).is_empty(),
        "the survey predicts no edge: {:?}",
        survey.targets
    );

    erase(&pool, w.profile).await;

    assert_eq!(
        records_of(&pool, homed).await.len(),
        1,
        "the second act erases nothing again"
    );
    assert_eq!(
        edge_state(&pool, outside).await,
        (false, Some(SHARED.to_owned()))
    );
}

/// W11's label, asserted from S to T by an edge outside the estate and by one inside it.
const SHARED: &str = "depends on";

/// W12. Four edges on one pair, in assertion order: A (the first subject's estate) and B (a team)
/// with one label, C (the team) with another, and D (a second subject's estate) with a third. The
/// second subject is erased first, then the first, then the pair's target by a standalone resource
/// erasure. Each act numbers the labels no earlier act wrote after the highest sentinel already on
/// the pair, so the four sentinels are distinct: D 3, A 4, B 5, C 6.
///
/// FAILS IF the label numbering ranks the written sentinels among the labels instead: D's
/// `erased-label-3` then sits after C's label, A's act numbers A 1, and the resource act ranks B 2
/// and C 3, writing C onto D's `erased-label-3`, two labels of one pair on one sentinel.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_fresh_label_numbers_after_every_sentinel_written_on_its_pair(pool: PgPool) {
    let w = world(&pool).await;
    let second = world(&pool).await;
    let member = world(&pool).await;
    let team = team_context(&pool).await;
    let s = resource_in(&pool, &member, team, "alpha").await;
    let t = resource_in(&pool, &member, team, "beta").await;
    edge_in(&pool, &w, w.home, s, t, SHARED, EventContext::default()).await;
    edge_in(&pool, &member, team, s, t, SHARED, EventContext::default()).await;
    edge_in(
        &pool,
        &member,
        team,
        s,
        t,
        "a team label",
        EventContext::default(),
    )
    .await;
    edge_in(
        &pool,
        &second,
        second.home,
        s,
        t,
        "a second estate's label",
        EventContext::default(),
    )
    .await;

    erase(&pool, second.profile).await;
    erase(&pool, w.profile).await;
    let admin = operator(&pool).await;
    let outcome = execute_resource_erasure(
        &pool,
        None,
        &admin,
        ResourceErasureRequest {
            resource: t,
            also_strike_blobs: &[],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("the resource act runs");
    assert!(matches!(outcome, ResourceErasureOutcome::Completed(_)));

    let labels = pair_labels(&pool, s, t).await;
    assert_eq!(
        labels,
        [
            "erased-label-4",
            "erased-label-5",
            "erased-label-6",
            "erased-label-3"
        ]
        .map(|l| Some(l.to_owned()))
        .to_vec(),
        "each act numbers after the pair's written sentinels"
    );
}

/// The calls of the function `name` in the current transaction, which must have set
/// `track_functions`.
async fn calls_in_xact(conn: &mut sqlx::PgConnection, name: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(calls), 0)::bigint FROM pg_stat_xact_user_functions \
          WHERE funcname::text = $1",
    )
    .bind(name)
    .fetch_one(conn)
    .await
    .unwrap()
}

/// W13. Two estate-homed edges: one between two estate resources, which the act's resource
/// erasures fold and erase, and one between team resources, which only the edge step reaches. The
/// act is run by SQL in a transaction counting function calls, after a survey on the same state, so
/// the act's own survey makes the same calls and the rest are the edge step's.
///
/// FAILS IF the edge step derives the completeness of an edge touching a resource this act erased
/// (a second `_edge_erasure_complete` call, a whole trail derived under the edge locks for an edge
/// that is complete by construction), or stops deciding the team edge by it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_edge_step_passes_over_an_edge_this_acts_resource_erasures_reached(pool: PgPool) {
    let w = world(&pool).await;
    let op = world(&pool).await;
    let team = team_context(&pool).await;
    let r1 = resource(&pool, &w, "estate alpha").await;
    let r2 = resource(&pool, &w, "estate beta").await;
    let reached = edge(&pool, &w, r1, r2, LABEL, authored(REASONING)).await;
    let a = resource_in(&pool, &w, team, "team alpha").await;
    let b = resource_in(&pool, &w, team, "team beta").await;
    let stepped = edge(
        &pool,
        &w,
        a,
        b,
        "the team pair's label",
        authored("its reasoning"),
    )
    .await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL track_functions = 'all'")
        .execute(&mut *tx)
        .await
        .expect("count function calls in this transaction");
    sqlx::query("SELECT principal_erasure_survey_plan($1)")
        .bind(w.profile)
        .execute(&mut *tx)
        .await
        .expect("the survey");
    let survey = calls_in_xact(&mut tx, "_edge_erasure_complete").await;
    sqlx::query("SELECT principal_erasure_execute($1, $2, $3, $4)")
        .bind(w.profile)
        .bind(op.profile)
        .bind(op.emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await
        .expect("the act");
    let step = calls_in_xact(&mut tx, "_edge_erasure_complete").await - 2 * survey;
    let recorded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'edge_erased' AND ev.payload ->> 'subject_id' = ANY($1)",
    )
    .bind(vec![reached.uuid().to_string(), stepped.uuid().to_string()])
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(
        step, 1,
        "the edge step tests the team edge alone ({survey} survey calls)"
    );
    assert_eq!(recorded, 1, "the edge step erases the team edge alone");
}

/// Wait until a backend running `statement` is blocked on a lock, by `blocker` when one is named.
/// Panics if none is within ten seconds, or if `task` finished first, which means it never waited.
async fn await_waiting<T>(
    pool: &PgPool,
    blocker: Option<i32>,
    statement: &str,
    task: &tokio::task::JoinHandle<T>,
    what: &str,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND ($1::int IS NULL OR $1 = ANY (pg_blocking_pids(pid))) \
                               AND query ILIKE $2)",
        )
        .bind(blocker)
        .bind(statement)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(!task.is_finished(), "{what}: it finished without waiting");
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: it never waited"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// W14. The subject has two `@me` contexts; the edge lives in the one with the higher id, between
/// team resources, so only the edge step reaches it. A side transaction holds the lower context
/// `FOR SHARE`, so the act, having erased the edge, waits at its estate-context lock, which takes
/// the lower context first. Meanwhile the subject reassigns the edge's context to a team it owns,
/// through `context_reassign`, the door a context leaves the estate by. The edge step does not hold
/// the edge's home context (ruling 12 of decision 01a1f6e8-81bd-758c-be87-a6824b66352a), so the
/// reassign commits while the act waits, and the commit check
/// (`edge_erasure_requires_an_erased_owner`) refuses the record, whose edge is no longer homed in
/// the estate: the act fails as a whole and the service answers 500. Nothing it did persists, and
/// the act run again completes, leaving the reassigned context's edge live.
///
/// FAILS IF the reassign waits on the act (a lock on the edge's home context taken before the
/// act's context lock), or if the act commits a record for an edge homed outside the estate, or
/// if any of the failed act's writes persist, or if the re-run does not complete.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_reassign_during_the_act_fails_it_and_a_rerun_completes(pool: PgPool) {
    let w = world(&pool).await;
    let second = test_support::context::insert_personal(&pool, w.profile, "second", "second").await;
    let (low, high) = if w.home.uuid() < second {
        (w.home.uuid(), second)
    } else {
        (second, w.home.uuid())
    };
    let target = test_support::team::insert(&pool, &format!("team-{}", Uuid::now_v7())).await;
    test_support::team::add_member(&pool, target, w.profile, "owner").await;
    let team = ContextId::from(
        test_support::context::insert_team(&pool, target, "shared", "shared").await,
    );
    let a = resource_in(&pool, &w, team, "alpha").await;
    let b = resource_in(&pool, &w, team, "beta").await;
    let e = edge_in(
        &pool,
        &w,
        ContextId::from(high),
        a,
        b,
        LABEL,
        authored(REASONING),
    )
    .await;
    let admin = operator(&pool).await;
    let quoted = ledger_quoting(&pool, LABEL).await;
    assert!(
        quoted > 0,
        "the edge's label is on the ledger before the act"
    );

    let mut held = pool.begin().await.unwrap();
    let holder: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM kb_contexts WHERE id = $1 FOR SHARE")
        .bind(low)
        .execute(&mut *held)
        .await
        .unwrap();
    let pool2 = pool.clone();
    let subject = w.profile;
    let act = tokio::spawn(async move {
        execute_erasure(
            &pool2,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
            &Default::default(),
        )
        .await
    });
    await_waiting(
        &pool,
        Some(holder),
        "%principal_erasure_execute%",
        &act,
        "the act at its estate-context lock",
    )
    .await;

    sqlx::query_scalar::<_, Uuid>("SELECT context_reassign($1, $2)")
        .bind(json!({
            "context_id": high,
            "to_owner_table": "kb_teams",
            "to_owner_id": target,
        }))
        .bind(w.emitter)
        .fetch_one(&pool)
        .await
        .expect("the reassign commits while the act waits at its context lock");
    assert!(
        !act.is_finished(),
        "the act is still waiting at its context lock"
    );

    held.rollback().await.unwrap();
    let err = act
        .await
        .unwrap()
        .expect_err("the commit check refuses a record whose edge left the estate");
    assert!(
        matches!(err, ApiError::Internal(ref m) if m == "principal erasure act failed"),
        "the service answers the incumbent 500: {err:?}"
    );

    assert!(
        records_of(&pool, e).await.is_empty(),
        "no edge_erased persists"
    );
    assert_eq!(
        edge_state(&pool, e).await,
        (false, Some(LABEL.to_owned())),
        "the edge is live and labelled, as before the act"
    );
    assert_eq!(
        ledger_quoting(&pool, LABEL).await,
        quoted,
        "no ledger rewrite persists"
    );
    let tombstoned: bool =
        sqlx::query_scalar("SELECT tombstoned_at IS NOT NULL FROM kb_profiles WHERE id = $1")
            .bind(w.profile)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!tombstoned, "the subject is not tombstoned");
    let recorded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'principal_erased' AND ev.payload ->> 'subject_id' = $1::text",
    )
    .bind(w.profile)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(recorded, 0, "no principal_erased persists");

    let completion = erase(&pool, w.profile).await;
    assert!(
        edge_outcomes(&completion.targets).is_empty(),
        "the re-run reaches no edge, the context having left the estate: {:?}",
        completion.targets
    );
    assert!(
        records_of(&pool, e).await.is_empty(),
        "the re-run erases no edge"
    );
    assert_eq!(
        edge_state(&pool, e).await,
        (false, Some(LABEL.to_owned())),
        "the reassigned context's edge stays live"
    );
    let owner: (String, Uuid) =
        sqlx::query_as("SELECT owner_table, owner_id FROM kb_contexts WHERE id = $1")
            .bind(high)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        owner,
        ("kb_teams".to_owned(), target),
        "the context stays with the team"
    );
}
