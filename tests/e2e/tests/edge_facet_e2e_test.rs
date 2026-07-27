#![cfg(feature = "test-db")]

//! End-to-end coverage for edge-owned properties — facets whose owner is a relationship
//! (task `019fa03a-913b-7141-a173-1c804d9b7ccd`).
//!
//! Drives the real `temper-client` → `temper-api` → Postgres path, which is the wire the CLI's
//! `temper edge facet` / `temper edge facets` uses. The lower layers are covered by
//! `crates/temper-substrate/tests/edge_owned_properties.rs`; what only this level can show is that
//! the two new routes, their authorization gate, and the fold cascade compose correctly through
//! the deployed surface.
//!
//! **The scenario is the one the feature exists for.** A task created with `--goal` projects a live
//! `advances` edge. The clause a task witnesses qualifies *that link*, not the task and not the
//! goal — so it belongs on the edge. Retracting the goal must take the clause citation with it,
//! because a citation outliving the link it qualifies is precisely the divergence edge facets were
//! chosen to make unrepresentable.

mod common;

use temper_core::types::facet_requests::EdgeFacetSetRequest;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_workflow::types::resource::ResourceUpdateRequest;
use uuid::Uuid;

/// Seed a resource via the API client, optionally linked to a goal. Mirrors
/// `goal_edge_e2e_test.rs`'s helper — the suite's per-file convention.
async fn seed(
    client: &temper_client::TemperClient,
    context: &str,
    doc_type: &str,
    slug: &str,
    goal: Option<Uuid>,
) -> Uuid {
    let mut managed = serde_json::Map::new();
    managed.insert("temper-mode".to_string(), serde_json::json!("build"));
    managed.insert("temper-effort".to_string(), serde_json::json!("small"));

    let payload = IngestPayload {
        segmented: None,
        title: format!("Edge facet e2e {slug}"),
        origin_uri: format!("kb://{context}/{doc_type}/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: doc_type.to_string(),
        goal,
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: Some(serde_json::Value::Object(managed)),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    let row = client
        .ingest()
        .create(&payload)
        .await
        .expect("seed resource via client");
    Uuid::from(row.id)
}

/// The handle of the `advances` edge `--goal` projected from this resource.
async fn advances_edge(client: &temper_client::TemperClient, task: Uuid) -> Uuid {
    let edges = client
        .resources()
        .edges(task)
        .await
        .expect("list the task's edges");
    edges
        .into_iter()
        .find(|e| e.label == "advances")
        .expect("the --goal create projected an advances edge")
        .edge_id
        .into()
}

/// A task's clause citation rides the `advances` edge, and is retracted with it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_clause_citation_rides_the_advances_edge_and_dies_with_it(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("edgefacet", None)
        .await
        .expect("create edgefacet context");

    let goal = seed(&app.client, "edgefacet", "goal", "the-goal", None).await;
    let task = seed(&app.client, "edgefacet", "task", "the-task", Some(goal)).await;
    let edge = advances_edge(&app.client, task).await;

    // ── the write: a clause citation on the link ──────────────────────────────────────────────
    let clause = serde_json::json!({ "clause": "decomposition-preserves-rigor" });
    app.client
        .facets()
        .set_on_edge(
            edge,
            &EdgeFacetSetRequest {
                values: clause.clone(),
                weight: 1.0,
                act: Default::default(),
            },
        )
        .await
        .expect("set a facet on the advances edge");

    // ── the read ──────────────────────────────────────────────────────────────────────────────
    let read = app
        .client
        .facets()
        .list_for_edge(edge)
        .await
        .expect("read the edge's facets");
    assert_eq!(read.edge_handle, edge);
    assert_eq!(
        read.facets.len(),
        1,
        "exactly the facet just written: {:?}",
        read.facets
    );
    assert_eq!(read.facets[0].value, clause);
    assert_eq!(read.facets[0].weight, 1.0);

    // ── the cascade: retracting the goal takes the citation with it ────────────────────────────
    // `clear_goal` folds the `advances` edge. If the cascade were missing, the clause would still
    // be readable here — a live citation of a link that no longer exists.
    app.client
        .resources()
        .update(
            task,
            &ResourceUpdateRequest {
                clear_goal: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("clear the goal, folding the advances edge");

    // The read now 404s, because `edges_visible_to` is `WHERE NOT e.is_folded` — a folded edge is
    // not visible, so neither are its facets. That is the incumbent predicate doing its job, not a
    // special case for facets.
    let after = app.client.facets().list_for_edge(edge).await;
    assert!(
        after.is_err(),
        "a folded edge is invisible, so its facets are unreadable: {after:?}"
    );

    // **This is the assertion that discriminates.** The 404 above comes from edge *visibility*, so
    // it would hold even if the cascade had never fired and a live clause row were sitting in the
    // table. Read the row directly: it must be folded, and folded BY the goal-clearing act.
    let (is_folded, retired_by_the_fold): (bool, bool) = sqlx::query_as(
        "SELECT p.is_folded, (p.last_event_id = e.last_event_id)
           FROM kb_properties p JOIN kb_edges e ON e.id = p.owner_id
          WHERE p.owner_table = 'kb_edges' AND p.owner_id = $1",
    )
    .bind(edge)
    .fetch_one(&pool)
    .await
    .expect("the property row is retained, not deleted");

    assert!(
        is_folded,
        "folding the advances edge must retract the clause citation it carried"
    );
    assert!(
        retired_by_the_fold,
        "the fold event must be what retired the citation, so the trail records which act did it"
    );
}

/// A profile that cannot modify the edge cannot facet it, and nothing is written.
///
/// The gate is `check_edge_mutable` — the same one governing retype/reweight/fold — reached by
/// `DbBackend::set_facet` dispatching on the typed owner. This is the clause that would silently
/// not exist if the edge arm had reused the resource gate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_owner_cannot_facet_someone_elses_edge(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("edgefacetauthz", None)
        .await
        .expect("create context");

    let goal = seed(&app.client, "edgefacetauthz", "goal", "goal", None).await;
    let task = seed(&app.client, "edgefacetauthz", "task", "task", Some(goal)).await;
    let edge = advances_edge(&app.client, task).await;

    // A provisioned, approved second profile — a legitimate principal, just not this edge's.
    let stranger_token = common::generate_second_user_jwt();
    common::provision_and_approve_second(&app).await;

    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/relationships/{edge}/facets")))
        .bearer_auth(&stranger_token)
        .json(&EdgeFacetSetRequest {
            values: serde_json::json!({ "clause": "not-yours" }),
            weight: 1.0,
            act: Default::default(),
        })
        .send()
        .await
        .expect("stranger facet request");

    let status = resp.status();
    assert!(
        status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::FORBIDDEN,
        "a stranger must not be able to facet another profile's edge, got {status}"
    );

    // Auth-before-writes: the refusal must leave no row behind.
    let written: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_properties WHERE owner_table = 'kb_edges'")
            .fetch_one(&pool)
            .await
            .expect("count edge-owned properties");
    assert_eq!(written, 0, "a denied facet set must write nothing");
}
