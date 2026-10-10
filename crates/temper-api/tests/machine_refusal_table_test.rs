//! Every act a machine is refused, sent by a machine, through the real app.
//!
//! The operations come from [`temper_api::routes::openapi_spec`], restricted to those that declare
//! `security` (every authenticated group; the public health check declares none), minus
//! [`ADMITTED`]: the machine-admitting handlers, the ones taking `AnyPrincipal`. A route added to
//! an authenticated group is therefore in this table without an edit.
//!
//! [`ADMITTED`] is keyed by method and path, not by handler name: a handler's OpenAPI identity is
//! its method and path, while about a third of them override `operation_id`. It is the same set
//! `.github/scripts/audit-route-auth.sh` pins as `ANY_PRINCIPAL_BASELINE` (check (f)), and the two
//! tests below hold it from both sides: a refused act newly admitted answers a machine without the
//! refusal and fails the first; an admitted act newly refused answers with it and fails the second.
//!
//! The machine holds approved standing. On the gated tier the refusal comes from `AuthUser`, which
//! runs after `require_system_access`, so an unapproved machine would meet the system-access
//! refusal instead and the table would witness the wrong gate.
#![cfg(feature = "test-db")]

mod common;

use std::collections::BTreeSet;

use serde_json::Value;
use sqlx::PgPool;
use temper_services::auth::MACHINE_PRINCIPAL_REFUSAL;
use uuid::Uuid;

const MACHINE_CLIENT: &str = "refusal-table-agent";

/// The operations a machine may reach, by method and path.
const ADMITTED: &[(&str, &str)] = &[
    ("DELETE", "/api/blobs/{id}"),    // handlers::blobs::delete
    ("DELETE", "/api/contexts/{id}"), // handlers::contexts::delete
    (
        "DELETE",
        "/api/relationships/{edge_handle}/facets/{property_id}",
    ), // handlers::facets::retract_edge_facet
    ("DELETE", "/api/resources/{id}"), // handlers::resources::delete
    ("GET", "/api/auditor/sweep"),    // handlers::auditor::sweep
    ("GET", "/api/blobs"),            // handlers::blobs::list
    ("GET", "/api/blobs/uploads/{id}"), // handlers::blobs::upload_progress
    ("GET", "/api/blobs/{id}"),       // handlers::blobs::get
    ("GET", "/api/blobs/{id}/relations"), // handlers::blobs::relations
    ("GET", "/api/cognitive-maps"),   // handlers::cognitive_maps::list
    ("GET", "/api/cognitive-maps/{id}"), // handlers::cognitive_maps::show
    ("GET", "/api/cognitive-maps/{id}/analytics"), // handlers::cognitive_maps::analytics
    ("GET", "/api/cognitive-maps/{id}/materialize-delta"), // handlers::cognitive_maps::materialize_delta
    ("GET", "/api/cognitive-maps/{id}/region-metrics"), // handlers::cognitive_maps::region_metrics
    ("GET", "/api/cognitive-maps/{id}/shape"),          // handlers::cognitive_maps::shape
    ("GET", "/api/cognitive-maps/{id}/shapes"), // handlers::data_artifact_shapes::list_cogmap_shapes
    ("GET", "/api/contexts"),                   // handlers::contexts::list
    ("GET", "/api/contexts/resolve"),           // handlers::contexts::resolve
    ("GET", "/api/contexts/{id}"),              // handlers::contexts::get
    ("GET", "/api/contexts/{id}/analytics"),    // handlers::contexts::analytics
    ("GET", "/api/contexts/{id}/materialize-delta"), // handlers::contexts::materialize_delta
    ("GET", "/api/contexts/{id}/region-metrics"), // handlers::contexts::region_metrics
    ("GET", "/api/contexts/{id}/shape"),        // handlers::contexts::shape
    ("GET", "/api/contexts/{id}/shapes"),       // handlers::data_artifact_shapes::list_shapes
    ("GET", "/api/data-artifacts/{artifact_id}"), // handlers::data_artifacts::get_by_id
    ("GET", "/api/events/{kb_context_id}/cursor"), // handlers::events::cursor
    ("GET", "/api/graph/cogmaps/{id}/panorama"), // handlers::graph::cogmap_panorama
    ("GET", "/api/graph/contexts/composition"), // handlers::graph::context_composition
    ("GET", "/api/graph/contexts/panorama"),    // handlers::graph::context_panorama
    ("GET", "/api/graph/elements/{kind}/{id}/trail"), // handlers::events::element_trail
    ("GET", "/api/graph/entry"),                // handlers::graph::entry
    ("GET", "/api/graph/home"),                 // handlers::graph::atlas_home
    ("GET", "/api/graph/regions/composition"),  // handlers::graph::region_composition
    ("GET", "/api/graph/traverse"),             // handlers::graph::traverse
    ("GET", "/api/invocations"),                // handlers::invocations::list
    ("GET", "/api/invocations/{id}"),           // handlers::invocations::show
    ("GET", "/api/profile"),                    // handlers::profiles::get
    ("GET", "/api/relationships/{edge_handle}/facets"), // handlers::facets::list_edge_facets
    ("GET", "/api/resources"),                  // handlers::resources::list
    ("GET", "/api/resources/{id}"),             // handlers::resources::get
    ("GET", "/api/resources/{id}/artifacts"),   // handlers::data_artifacts::list
    ("GET", "/api/resources/{id}/artifacts/{artifact_id}"), // handlers::data_artifacts::get
    ("GET", "/api/resources/{id}/blocks"),      // handlers::segments::list_blocks_handler
    ("GET", "/api/resources/{id}/blocks/{block_id}"), // handlers::resources::read_block
    ("GET", "/api/resources/{id}/citation-audits"), // handlers::citation_audits::list
    ("GET", "/api/resources/{id}/connections"), // handlers::edges::list_connections
    ("GET", "/api/resources/{id}/content"),     // handlers::resources::get_content
    ("GET", "/api/resources/{id}/edges"),       // handlers::edges::list
    ("GET", "/api/resources/{id}/evidence"),    // handlers::evidence::evidence
    ("GET", "/api/resources/{id}/facets"),      // handlers::facets::list_resource_facets
    ("GET", "/api/resources/{id}/lineage"),     // handlers::edges::lineage
    ("GET", "/api/resources/{id}/meta"),        // handlers::meta::get_meta
    ("GET", "/api/resources/{id}/provenance"),  // handlers::resources::provenance
    ("GET", "/api/schema/doc-types"),           // handlers::schema::list_doc_types
    ("GET", "/api/schema/doc-types/{name}"),    // handlers::schema::describe_doc_type
    ("GET", "/api/schema/open-meta"),           // handlers::schema::describe_open_meta
    ("GET", "/api/shapes/{shape_id}"),          // handlers::data_artifact_shapes::get_shape
    ("GET", "/api/steward/candidates"),         // handlers::steward::candidates
    ("GET", "/api/steward/sweep"),              // handlers::steward::sweep
    ("GET", "/api/steward/{cogmap}/delta"),     // handlers::steward::delta
    ("GET", "/api/teams"),                      // handlers::teams::list
    ("GET", "/api/teams/{id}"),                 // handlers::teams::detail
    ("PATCH", "/api/resources/{id}"),           // handlers::resources::update
    ("POST", "/api/auditor/dispatch"),          // handlers::auditor::dispatch
    ("POST", "/api/auditor/{cogmap}/complete"), // handlers::auditor::complete
    ("POST", "/api/blobs"),                     // handlers::blobs::commit
    ("POST", "/api/blobs/uploads"),             // handlers::blobs::begin_upload
    ("POST", "/api/blobs/uploads/{id}/finalize"), // handlers::blobs::finalize_upload
    ("POST", "/api/blobs/uploads/{id}/segments"), // handlers::blobs::append_segment
    ("POST", "/api/blobs/{id}/relations"),      // handlers::blobs::relate
    ("POST", "/api/citation-audits"),           // handlers::citation_audits::record_for_block
    ("POST", "/api/cogmaps/{id}/graph/slice"),  // handlers::graph::cogmap_neighborhood_slice
    ("POST", "/api/cognitive-maps"),            // handlers::cognitive_maps::genesis
    ("POST", "/api/cognitive-maps/{id}/materialize"), // handlers::cognitive_maps::materialize
    ("POST", "/api/cognitive-maps/{id}/shapes"), // handlers::data_artifact_shapes::declare_cogmap_shape
    ("POST", "/api/contexts"),                   // handlers::contexts::create
    ("POST", "/api/contexts/{id}/materialize"),  // handlers::contexts::materialize
    ("POST", "/api/contexts/{id}/rename"),       // handlers::contexts::rename
    ("POST", "/api/contexts/{id}/restore"),      // handlers::contexts::restore
    ("POST", "/api/contexts/{id}/shapes"),       // handlers::data_artifact_shapes::declare_shape
    ("POST", "/api/facets"),                     // handlers::facets::set_facet
    ("POST", "/api/ingest"),                     // handlers::ingest::create
    ("POST", "/api/invocations"),                // handlers::invocations::open
    ("POST", "/api/invocations/{id}/close"),     // handlers::invocations::close
    ("POST", "/api/query"),                      // handlers::query::query
    ("POST", "/api/relationships"),              // handlers::edges::assert
    ("POST", "/api/relationships/{edge_handle}/facets"), // handlers::facets::set_edge_facet
    ("POST", "/api/relationships/{edge_handle}/fold"), // handlers::edges::fold
    ("POST", "/api/relationships/{edge_handle}/retype"), // handlers::edges::retype
    ("POST", "/api/relationships/{edge_handle}/reweight"), // handlers::edges::reweight
    ("POST", "/api/resources"),                  // handlers::resources::create
    ("POST", "/api/resources/reblock"),          // handlers::reblock::reblock
    ("POST", "/api/resources/{id}/artifacts"),   // handlers::data_artifacts::commit
    ("POST", "/api/resources/{id}/blocks"),      // handlers::segments::append_block_handler
    ("POST", "/api/resources/{id}/citation-audits"), // handlers::citation_audits::record
    ("POST", "/api/resources/{id}/finalize"),    // handlers::segments::finalize_handler
    ("POST", "/api/resources/{id}/provenance"),  // handlers::resources::annotate
    ("POST", "/api/search"),                     // handlers::search::search
    ("POST", "/api/steward/dispatch"),           // handlers::steward::dispatch
    ("POST", "/api/steward/{cogmap}/watermark"), // handlers::steward::advance
    ("PUT", "/api/cognitive-maps/{id}"),         // handlers::cognitive_maps::reconcile
    ("PUT", "/api/ingest/{id}"),                 // handlers::ingest::update
    ("PUT", "/api/resources/{id}/meta"),         // handlers::meta::update_meta
];

/// Every authenticated operation in the published contract, as `(METHOD, path template)`.
fn authenticated_operations() -> BTreeSet<(String, String)> {
    let spec = serde_json::to_value(temper_api::routes::openapi_spec()).expect("spec serializes");
    let mut out = BTreeSet::new();
    for (path, item) in spec["paths"].as_object().expect("paths") {
        for (method, op) in item.as_object().expect("path item") {
            if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                continue;
            }
            let secured = op["security"].as_array().is_some_and(|s| !s.is_empty());
            if secured {
                out.insert((method.to_uppercase(), path.clone()));
            }
        }
    }
    out
}

fn admitted() -> BTreeSet<(String, String)> {
    ADMITTED
        .iter()
        .map(|(m, p)| (m.to_string(), p.to_string()))
        .collect()
}

/// A path template with every parameter filled by the nil UUID: well-shaped for a `Uuid` path
/// extractor and for a string one, so no extractor answers `400` before the identity does.
fn concrete(template: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}').expect("closed parameter") + open;
        out.push_str(&rest[..open]);
        out.push_str(&Uuid::nil().to_string());
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A registered machine with approved standing; returns its bearer.
async fn approved_machine(pool: &PgPool) -> String {
    let profile = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
    )
    .bind(profile)
    .bind(MACHINE_CLIENT)
    .execute(pool)
    .await
    .expect("seed machine profile");
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'test', $2, $2)",
    )
    .bind(MACHINE_CLIENT)
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed machine registration");
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'approved')")
        .bind(profile)
        .execute(pool)
        .await
        .expect("approve machine");
    common::generate_machine_jwt(MACHINE_CLIENT)
}

/// Send `(method, template)` as the machine; returns the status and the parsed body.
async fn send(app: &common::TestApp, token: &str, method: &str, template: &str) -> (u16, Value) {
    let url = app.url(&concrete(template));
    let req = match method {
        "GET" => app.client.get(url),
        "DELETE" => app.client.delete(url),
        "POST" => app.client.post(url).json(&serde_json::json!({})),
        "PUT" => app.client.put(url).json(&serde_json::json!({})),
        "PATCH" => app.client.patch(url).json(&serde_json::json!({})),
        other => panic!("unexpected method {other}"),
    };
    let resp = req.bearer_auth(token).send().await.expect("request");
    let status = resp.status().as_u16();
    let body = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

fn is_machine_refusal(status: u16, body: &Value) -> bool {
    status == 403
        && body["error"]["code"] == temper_core::error::FORBIDDEN_DETAIL_CODE
        && body["error"]["message"].as_str() == Some(MACHINE_PRINCIPAL_REFUSAL)
}

/// Every refused act answers an approved machine `403` `FORBIDDEN_DETAIL` with the fixed sentence,
/// exactly: `invitation_service` and `blob_service` emit `FORBIDDEN_DETAIL` too, and a different
/// refusal must not satisfy this.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_refused_act_answers_a_machine_with_the_refusal(pool: PgPool) {
    let token = approved_machine(&pool).await;
    let app = common::setup_test_app(pool).await;

    let operations = authenticated_operations();
    let admitted = admitted();
    let stale: Vec<_> = admitted.difference(&operations).collect();
    assert!(
        stale.is_empty(),
        "ADMITTED names operations the contract does not have: {stale:?}"
    );

    let refused: Vec<_> = operations.difference(&admitted).collect();
    assert!(!refused.is_empty(), "the table must cover something");

    let mut wrong = Vec::new();
    for (method, template) in &refused {
        let (status, body) = send(&app, &token, method, template).await;
        if !is_machine_refusal(status, &body) {
            wrong.push(format!("{method} {template} -> {status} {body}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} refused acts did not answer a machine with the refusal:\n{}",
        wrong.len(),
        refused.len(),
        wrong.join("\n")
    );
}

/// No admitted act answers an approved machine with the refusal. What it answers instead (a `404`
/// for a nil id, a `400` or `422` for an empty body) is not this test's concern: only that the
/// machine was not turned away as a machine.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_admitted_act_answers_a_machine_with_the_refusal(pool: PgPool) {
    let token = approved_machine(&pool).await;
    let app = common::setup_test_app(pool).await;

    let mut wrong = Vec::new();
    for (method, template) in ADMITTED {
        let (status, body) = send(&app, &token, method, template).await;
        if is_machine_refusal(status, &body) {
            wrong.push(format!("{method} {template}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "admitted acts refused a machine:\n{}",
        wrong.join("\n")
    );
}
