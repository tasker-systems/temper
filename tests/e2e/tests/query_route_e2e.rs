//! `POST /api/query` through the real route — the door, knocked on.
//!
//! **What A2 already took, so this does not repeat it.**
//! `crates/temper-services/tests/query_run_composition_test.rs::server_side_embedding` drives a
//! find-about stage through `prepare → compile → execute` against a real corpus with a caller that
//! supplies no vector. Calling that again here would buy nothing.
//!
//! What was still uncovered is **the route**: auth, the system-access gate, a `Composition`
//! deserialized from real JSON rather than hand-built in Rust, and — the reason Task B1.1 exists —
//! the shape of the 400 body. Those are reachable only through HTTP.
//!
//! `test-embed` gated, and that gate is load-bearing rather than incidental: the happy path drives
//! a **find-about** stage, so it needs the real ONNX model both to ingest chunks with true
//! embeddings and for the server to embed the stage's question. A run scoped `--features test-db`
//! alone compiles this file to nothing and reads green. Use `cargo make test-e2e-embed`.
#![cfg(all(feature = "test-db", feature = "test-embed"))]

mod common;

use reqwest::StatusCode;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_core::types::query::{
    ActInvocation, ActName, Composition, Intention, OutcomeDeclaration, ReturnSpec,
    StageDisposition, StageName, StageNode, StageOutput,
};

/// Ingest a resource whose chunks carry REAL bge embeddings, so a find-about stage has a vector
/// space to match against. Mirrors `server_query_embed_test.rs::ingest_semantic`.
async fn ingest_semantic(app: &common::E2eTestApp, title: &str, slug: &str, content: &str) {
    let packed = temper_ingest::pipeline::prepare_markdown(content).expect("prepare_markdown");
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://query-route/{slug}"),
        context_ref: "@me/qroute".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(content)),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&packed).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");
}

fn find_about(stage: &str, query: &str) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(stage).unwrap(),
        act: ActName::FindAboutAnywhere,
        // No embedding — this is the raw-HTTP caller, the class of client that structurally cannot
        // compute one. The server must embed it.
        intention: Some(Intention {
            query: query.to_string(),
            embedding: None,
        }),
        inputs: vec![],
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    })
}

/// A composition returning every stage it declares.
fn returning_all(stages: Vec<StageNode>) -> Composition {
    Composition {
        outcome: OutcomeDeclaration {
            returns: stages
                .iter()
                .map(|n| ReturnSpec {
                    stage: n.name().clone(),
                    with: vec![],
                })
                .collect(),
        },
        stages,
    }
}

/// POST a composition as JSON, exactly as a `curl` caller would — serialized, over the wire, and
/// deserialized by the route rather than handed over in Rust.
async fn post_query(
    app: &common::E2eTestApp,
    composition: &Composition,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .post(app.url("/api/query"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(composition)
        .send()
        .await
        .expect("POST /api/query failed");
    let status = resp.status();
    (status, resp.json().await.expect("response body is JSON"))
}

/// The happy path, end to end: a plan goes out as JSON and comes back hydrated.
///
/// The assertion is that the stage **answered with a real row**, not merely that no refusal came
/// back — a server that failed to embed would refuse `EmbeddingUnavailable`, and an assertion on
/// the absence of an error would be satisfied by a stage that answered empty for a different
/// reason. Only a hydrated title proves compile → execute → hydration ran through the route.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_composition_posted_as_json_comes_back_hydrated(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("qroute", None)
        .await
        .expect("context create");

    ingest_semantic(
        &app,
        "Container Scheduling Primer",
        "container-scheduling-primer",
        "Pods, replicas, and self-healing workloads are placed and rescheduled automatically by \
         the control plane.",
    )
    .await;

    let composition = returning_all(vec![find_about("about", "kubernetes deployment")]);
    let (status, body) = post_query(&app, &composition).await;
    assert_eq!(status, StatusCode::OK, "route refused a valid plan: {body}");

    // Deserialize into the published response type — which also asserts the wire shape the SDKs
    // were generated against actually round-trips.
    let response: temper_core::types::query::QueryResponse = serde_json::from_value(body.clone())
        .unwrap_or_else(|e| panic!("QueryResponse parse: {e}\n{body}"));

    let stage = &response.returned[&StageName::parse("about").unwrap()];
    assert_eq!(
        stage.disposition,
        StageDisposition::Answered,
        "the find-about stage did not answer; refusal: {:?}",
        stage.refusal
    );
    let hits = match &stage.produced {
        StageOutput::Resources { hits } => hits,
        other => panic!("expected resources, got {other:?}"),
    };
    assert!(
        hits.iter()
            .any(|h| h.resource.title == "Container Scheduling Primer"),
        "the seeded resource must come back hydrated through the route; got {:?}",
        hits.iter().map(|h| &h.resource.title).collect::<Vec<_>>()
    );

    // EVERY stage is traced, including ones whose rows were not returned.
    assert!(
        !response.trace.stages.is_empty(),
        "a composition answered with no trace is a black box"
    );
}

/// **The 400 carries MORE THAN ONE refusal.**
///
/// This is the property Task B1.1 exists to make expressible, and the only end-to-end witness that
/// `details` survived the `oneOf` widening. A single-refusal assertion would pass against a body
/// that truncates the list — which is exactly the failure `validate`'s "every refusal, not the
/// first" rule exists to prevent, and which would be invisible from a one-refusal plan.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refused_plan_returns_400_with_every_refusal_in_one_round_trip(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // Two find-about stages, neither carrying an intention. Each is independently unrunnable, so a
    // validator that stopped at the first would answer with one.
    let mut first = find_about("one", "");
    let mut second = find_about("two", "");
    for node in [&mut first, &mut second] {
        if let StageNode::Act(act) = node {
            act.intention = None;
        }
    }
    let composition = returning_all(vec![first, second]);

    let (status, body) = post_query(&app, &composition).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unrunnable plan must be a CALLER fault: {body}"
    );
    assert_eq!(
        body["error"]["code"], "PLAN_REFUSED",
        "the refusal body must be identifiable by code, not by sniffing its shape: {body}"
    );

    let refusals = body["error"]["details"]["refusals"]
        .as_array()
        .unwrap_or_else(|| panic!("no `error.details.refusals` array in {body}"));
    assert!(
        refusals.len() >= 2,
        "every refusal must arrive in ONE round trip; got {}: {body}",
        refusals.len()
    );

    // Each refusal names the stage it attaches to, so a caller can repair the right one.
    let stages: Vec<&str> = refusals
        .iter()
        .filter_map(|r| r["stage"].as_str())
        .collect();
    assert!(
        stages.contains(&"one") && stages.contains(&"two"),
        "both unrunnable stages must be named; got {stages:?}"
    );

    // The refusal list is the published type, not an ad-hoc blob.
    let parsed: Vec<temper_core::types::query::validate::PlanRefusal> =
        serde_json::from_value(body["error"]["details"]["refusals"].clone())
            .expect("refusals deserialize as the published PlanRefusal");
    assert_eq!(parsed.len(), refusals.len());
}

/// The route sits behind the same gate every content-touching route does — asserted, not assumed,
/// because a door that answers an unauthenticated caller is a different door than the one designed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_route_refuses_an_unauthenticated_caller(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let composition = returning_all(vec![find_about("about", "anything")]);

    let resp = app
        .reqwest_client
        .post(app.url("/api/query"))
        .json(&composition)
        .send()
        .await
        .expect("POST /api/query failed");

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "the query door must not answer a caller with no token"
    );
}

/// **`find-about` -> `follow-from`, through the door, with provenance coming back.**
///
/// `[added — 2026-08-14]` The shape the whole act exists for: find things, then get their
/// neighbours and know which of the things you found each one came from. It is also the first
/// end-to-end witness that `follow-from` has a door at all — `registry.rs` declares `Serves` at CLI
/// and API as of this change, and a declaration nothing exercises is a claim.
///
/// **What only this layer can say.** The composition crosses HTTP as JSON, so `inputs` — a LIST
/// since the widening — has to deserialize with two entries carrying different relations, and
/// `via` has to survive `jsonb` -> `ViaEntry` -> `QueryResponse` -> the wire. Every layer below
/// asserts one hop of that; none of them asserts the whole chain.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_walk_seeded_by_a_find_returns_its_provenance_through_the_route(pool: sqlx::PgPool) {
    use temper_core::types::graph::{EdgeKind, Polarity};
    use temper_core::types::query::{IdKind, IdSet, StageInput, StageRelation};
    use temper_core::types::relationship_requests::AssertRelationshipRequest;

    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("qroute", None)
        .await
        .expect("context create");

    ingest_semantic(
        &app,
        "Container Scheduling Primer",
        "container-scheduling-primer",
        "Pods, replicas, and self-healing workloads are placed and rescheduled automatically by \
         the control plane.",
    )
    .await;
    ingest_semantic(
        &app,
        "Sourdough Starter Notes",
        "sourdough-starter-notes",
        "Feeding ratios, hydration, and the smell of a healthy levain after four days.",
    )
    .await;

    // The two resources, by title — the walk needs real ids and the ingest returns none here.
    let ids: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT id, title FROM kb_resources WHERE is_active ORDER BY created")
            .fetch_all(&app.pool)
            .await
            .expect("resources");
    let id_of = |t: &str| {
        ids.iter()
            .find(|(_, title)| title == t)
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("{t} was ingested; got {ids:?}"))
    };
    let primer = id_of("Container Scheduling Primer");
    let sourdough = id_of("Sourdough Starter Notes");

    // An edge with a label and an INVERSE polarity — the two fields a `via` entry reports that a
    // bare parent id could not, and inverse because it is the case a direction flag gets backwards.
    app.client
        .relationships()
        .assert(&AssertRelationshipRequest {
            source: primer.into(),
            target: sourdough.into(),
            edge_kind: EdgeKind::Contains,
            polarity: Polarity::Inverse,
            label: "digresses-to".to_string(),
            weight: 1.0,
            act: Default::default(),
        })
        .await
        .expect("edge assert");

    // The walk is seeded by the CALLER here rather than by the find stage's output, because what
    // this test is about is the route and the payload. The upstream-seeded form is exercised at the
    // compile and execute layers, where the emitted SQL can actually be read.
    let walk = StageNode::Act(ActInvocation {
        name: StageName::parse("neighbours").unwrap(),
        act: ActName::FollowFrom,
        intention: None,
        inputs: vec![StageInput::Caller {
            relation: StageRelation::Seed,
            ids: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids: vec![primer],
            },
        }],
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    });

    let (status, body) = post_query(&app, &returning_all(vec![walk])).await;
    assert_eq!(status, StatusCode::OK, "route refused a valid walk: {body}");

    let response: temper_core::types::query::QueryResponse = serde_json::from_value(body.clone())
        .unwrap_or_else(|e| panic!("QueryResponse parse: {e}\n{body}"));
    let stage = &response.returned[&StageName::parse("neighbours").unwrap()];
    assert_eq!(
        stage.disposition,
        StageDisposition::Answered,
        "the walk did not answer; refusal: {:?}",
        stage.refusal
    );

    let hits = match &stage.produced {
        StageOutput::Resources { hits } => hits,
        other => panic!("expected resources, got {other:?}"),
    };
    let hit = hits
        .iter()
        .find(|h| h.resource.id.uuid() == sourdough)
        .unwrap_or_else(|| {
            panic!(
                "the neighbour must come back; got {:?}",
                hits.iter().map(|h| &h.resource.title).collect::<Vec<_>>()
            )
        });

    assert_eq!(hit.via.len(), 1, "one edge reached it; got {:?}", hit.via);
    let entry = &hit.via[0];
    assert_eq!(entry.seed_id, primer, "which seed this neighbour came from");
    assert_eq!(
        entry.source_id, primer,
        "the edge's own source, as asserted"
    );
    assert_eq!(entry.target_id, sourdough, "and its own target");
    assert_eq!(entry.edge_kind, EdgeKind::Contains);
    assert_eq!(entry.label.as_deref(), Some("digresses-to"));
    assert_eq!(
        entry.polarity,
        Polarity::Inverse,
        "polarity survives the whole path — without it a `contains` edge reads backwards, which is \
         the majority case in prod"
    );

    // The seed itself is NOT a neighbour of itself: the walk scores only >=1-hop proximity.
    assert!(
        hits.iter().all(|h| h.resource.id.uuid() != primer),
        "a seed earns no hop-0 self-score"
    );

    // And a find stage in the same response carries no provenance — `via` is absent, not empty,
    // for an act whose `discloses` does not name it.
    let (find_status, find_body) = post_query(
        &app,
        &returning_all(vec![find_about("about", "kubernetes")]),
    )
    .await;
    assert_eq!(find_status, StatusCode::OK, "{find_body}");
    let find_json = find_body.to_string();
    assert!(
        !find_json.contains("\"via\""),
        "a find act discloses no origin, so the key is absent rather than an empty array; got \
         {find_body}"
    );
}

/// **What the deserializer boundary already answers with, for ANY unparseable body.**
///
/// `[added — 2026-08-14]` Written to check a claim I had made without checking: that refusing an
/// unknown key at the serde layer is "inconsistent" with the every-refusal-at-once contract. It is
/// not — the door has always answered malformed bodies from the extractor, and a wrong TYPE on a
/// known field takes exactly the same path. The refusal contract's promise is about plans that
/// PARSE and then fail validation.
///
/// Pinned because the answer decides a design choice (`deny_unknown_fields` versus capture-and-
/// refuse) and would otherwise have to be re-derived from axum's defaults by the next person.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unparseable_body_answers_from_the_extractor_not_the_refusal_contract(
    pool: sqlx::PgPool,
) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // A wrong TYPE on a known field — nothing to do with unknown keys.
    let bad_type = serde_json::json!({
        "outcome": {"returns": [{"stage": "hits", "with": []}]},
        "stages": [{"act": {"act": "find-exact", "name": "hits", "terms": "not-a-map"}}]
    });
    let resp = app
        .reqwest_client
        .post(app.url("/api/query"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&bad_type)
        .send()
        .await
        .expect("POST");
    let status = resp.status();
    let body = resp.text().await.expect("body");

    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "a body that cannot be parsed into a plan is refused: {status} {body}"
    );
    assert!(
        !body.contains("refusals"),
        "and it does NOT carry the every-refusal-at-once envelope — that promise is about plans \
         that parse. This is the pre-existing extractor boundary, which every type error already \
         takes: {body}"
    );
}
