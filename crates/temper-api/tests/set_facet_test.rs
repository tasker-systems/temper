#![cfg(feature = "test-db")]
//! T1 Sequence B Task B2 — `DbBackend::set_facet` over `writes::set_facet_with`.
//!
//! Exercises the backend write method directly (the same approach as
//! `act_authorship_test`): a facet set on a resource the caller owns
//! succeeds and returns the `kb_properties.id` the fire produced; a facet
//! set attempted by a non-owner profile is rejected with `Forbidden`
//! BEFORE any write (auth-before-write, WS2).

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, PropertyId};
use temper_core::types::property_owner::PropertyOwner;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CommandOutput, CreateResource, SetFacet, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

mod common;

async fn backend_with_context(pool: &PgPool, email: &str) -> (DbBackend, ContextId) {
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, email).await;
    (
        DbBackend::new(pool.clone(), ProfileId::from(profile)),
        ContextId::from(context),
    )
}

fn create_cmd(context: ContextId, slug: &str) -> CreateResource {
    CreateResource {
        resource_id: None,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(context),
        title: format!("Facet test {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://facet-{slug}")),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// A backend that owns one freshly created resource — the setup every grain test below shares.
async fn facet_fixture(pool: &PgPool) -> (DbBackend, temper_core::types::ids::ResourceId) {
    let (backend, context) = backend_with_context(pool, "facet-grain@example.com").await;
    let resource = backend
        .create_resource(create_cmd(context, "grain"))
        .await
        .expect("create")
        .value
        .id;
    (backend, resource)
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn set_facet_returns_property_id_and_gates_auth(pool: PgPool) {
    let (owner_backend, context) = backend_with_context(&pool, "facet-owner@example.com").await;
    let resource = owner_backend
        .create_resource(create_cmd(context, "owned"))
        .await
        .expect("owner create")
        .value
        .id;

    // Non-owner profile: rejected BEFORE any write.
    let (other_backend, _other_context) =
        backend_with_context(&pool, "facet-other@example.com").await;
    let denied = other_backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(resource),
            values: serde_json::json!({"k": "v"}),
            weight: 1.0,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "a non-owner facet set must be Forbidden (403): {denied:?}"
    );
    let property_count_after_deny: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_properties WHERE property_key = 'facet'")
            .fetch_one(&pool)
            .await
            .expect("property count after deny");
    assert_eq!(
        property_count_after_deny, 0,
        "the denied non-owner facet set must not have written anything"
    );

    // Owner: succeeds, returns the property ids.
    let CommandOutput {
        value: property_ids,
        ..
    } = owner_backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(resource),
            values: serde_json::json!({"k": "v"}),
            weight: 1.0,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("owner facet set must succeed");

    assert_eq!(
        property_ids.len(),
        1,
        "a one-key facet writes exactly one row: {property_ids:?}"
    );
    assert_ne!(
        property_ids[0],
        PropertyId::from(Uuid::nil()),
        "set_facet must return a real property id"
    );

    let (stored_key, stored_value): (String, serde_json::Value) =
        sqlx::query_as("SELECT property_key, property_value FROM kb_properties WHERE id = $1")
            .bind(property_ids[0].uuid())
            .fetch_one(&pool)
            .await
            .expect("the facet property row must exist");
    assert_eq!(stored_key, "facet");
    assert_eq!(stored_value, serde_json::json!({"k": "v"}));
}

/// A multi-key assert writes one row per inner key, and the ack names **every** one.
///
/// This is the shape the singular ack could not express: before the inner-key grain, `{a, b}` was a
/// single row and a single id, so "how many marks did that write" had no answer on the wire. The
/// assertion is on the returned ids *and* on the stored rows, because an ack that reported two ids
/// while storing one row would satisfy neither half alone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn set_facet_multi_key_writes_and_acks_one_row_per_inner_key(pool: PgPool) {
    let (owner_backend, resource) = facet_fixture(&pool).await;

    let CommandOutput {
        value: property_ids,
        ..
    } = owner_backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(resource),
            values: serde_json::json!({"status": "open", "as_of": "2026-07-30"}),
            weight: 0.85,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("owner facet set must succeed");

    assert_eq!(
        property_ids.len(),
        2,
        "a two-key facet must ack two rows: {property_ids:?}"
    );

    let stored: Vec<(serde_json::Value, f64)> = sqlx::query_as(
        "SELECT property_value, weight FROM kb_properties \
          WHERE property_key = 'facet' AND NOT is_folded AND owner_id = $1 \
          ORDER BY property_value::text",
    )
    .bind(uuid::Uuid::from(resource))
    .fetch_all(&pool)
    .await
    .expect("stored facet rows");

    assert_eq!(
        stored,
        vec![
            (serde_json::json!({"as_of": "2026-07-30"}), 0.85),
            (serde_json::json!({"status": "open"}), 0.85),
        ],
        "each inner key is its own one-key-object row, carrying the assert's weight"
    );
}

/// **Patch, not replace** — the invariant the whole grain change exists to make true.
///
/// Re-asserting one key folds that key's prior row and leaves every unnamed mark live *at its own
/// weight*. Written to fail against the append-only projector too: under append, `status` would have
/// two live rows rather than one, so this bites on both the old bug and a replace-shaped fix.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn re_asserting_one_facet_key_leaves_the_others_untouched(pool: PgPool) {
    let (owner_backend, resource) = facet_fixture(&pool).await;

    let set = |values: serde_json::Value, weight: f64| {
        let backend = &owner_backend;
        async move {
            backend
                .set_facet(SetFacet {
                    owner: PropertyOwner::resource(resource),
                    values,
                    weight,
                    act: ActContext::default(),
                    origin: Surface::ApiHttp,
                })
                .await
                .expect("facet set must succeed")
        }
    };

    set(serde_json::json!({"status": "open", "as_of": "X"}), 0.85).await;
    set(serde_json::json!({"status": "resolved"}), 1.0).await;

    let live: Vec<(serde_json::Value, f64)> = sqlx::query_as(
        "SELECT property_value, weight FROM kb_properties \
          WHERE property_key = 'facet' AND NOT is_folded AND owner_id = $1 \
          ORDER BY property_value::text",
    )
    .bind(uuid::Uuid::from(resource))
    .fetch_all(&pool)
    .await
    .expect("live facet rows");

    assert_eq!(
        live,
        vec![
            // untouched by the second assert — still live, still at ITS OWN weight
            (serde_json::json!({"as_of": "X"}), 0.85),
            // superseded in place: one live row for the key, not two
            (serde_json::json!({"status": "resolved"}), 1.0),
        ],
        "an assert must fold only the keys it names and never a sibling"
    );

    let folded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE property_key = 'facet' AND is_folded AND owner_id = $1",
    )
    .bind(uuid::Uuid::from(resource))
    .fetch_one(&pool)
    .await
    .expect("folded count");
    assert_eq!(folded, 1, "exactly the superseded `status` mark is folded");
}

/// The door refuses a double-encoded facet — the shape that produced 41 production rows.
///
/// Guard lives on the SQL wrapper, not the projector: replay calls the projector directly and must
/// keep projecting the history that already contains this shape.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_double_encoded_facet_value_is_refused_at_the_door(pool: PgPool) {
    let (owner_backend, resource) = facet_fixture(&pool).await;

    let refused = owner_backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(resource),
            // a STRING holding a serialized object, not an object
            values: serde_json::json!(r#"{"node_label": "domain"}"#),
            weight: 1.0,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await;

    assert!(
        refused.is_err(),
        "a non-object facet value must be refused, not stored: {refused:?}"
    );

    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE property_key = 'facet' AND owner_id = $1",
    )
    .bind(uuid::Uuid::from(resource))
    .fetch_one(&pool)
    .await
    .expect("row count");
    assert_eq!(rows, 0, "the refusal must not have written anything");
}

/// The backfill rebuilds pre-grain rows from their events — the statement that runs against
/// production, exercised against real old-grain data.
///
/// The migration's inline block ran once, on a database with no facet rows, so it proved nothing.
/// Here the projector's own output is collapsed back into the **pre-grain shape** — one row holding
/// the whole multi-key object, which is exactly what all 443 production rows look like — and the
/// repair is asked to recover from it.
///
/// It asserts the rebuild against the projector's semantics rather than against a hand-written
/// expectation, because that is the property the migration actually needs: the backfill must land
/// the state a replay would produce, not merely *a* plausible state.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_backfill_rebuilds_old_grain_rows_from_their_events(pool: PgPool) {
    let (owner_backend, resource) = facet_fixture(&pool).await;
    let owner_uuid = uuid::Uuid::from(resource);

    owner_backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(resource),
            values: serde_json::json!({"node_label": "fact", "status": "open"}),
            weight: 0.9,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("assert");

    let after_write: Vec<(serde_json::Value, f64)> = sqlx::query_as(
        "SELECT property_value, weight FROM kb_properties \
          WHERE property_key='facet' AND NOT is_folded AND owner_id=$1 \
          ORDER BY property_value::text",
    )
    .bind(owner_uuid)
    .fetch_all(&pool)
    .await
    .expect("rows after write");
    assert_eq!(after_write.len(), 2, "two marks to begin with");

    // Collapse to the PRE-GRAIN shape: delete the split rows and put back the single whole-object
    // row the old projector would have written, keeping the same asserting event so the rebuild has
    // real history to work from.
    let event_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT asserted_by_event_id FROM kb_properties \
          WHERE property_key='facet' AND owner_id=$1 LIMIT 1",
    )
    .bind(owner_uuid)
    .fetch_one(&pool)
    .await
    .expect("event id");

    sqlx::query("DELETE FROM kb_properties WHERE property_key='facet' AND owner_id=$1")
        .bind(owner_uuid)
        .execute(&pool)
        .await
        .expect("collapse");
    sqlx::query(
        "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, \
                                    asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'facet', $2, 0.9, $3, $3)",
    )
    .bind(owner_uuid)
    .bind(serde_json::json!({"node_label": "fact", "status": "open"}))
    .bind(event_id)
    .execute(&pool)
    .await
    .expect("insert pre-grain row");

    let pre_grain: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE property_key='facet' AND owner_id=$1",
    )
    .bind(owner_uuid)
    .fetch_one(&pool)
    .await
    .expect("pre-grain count");
    assert_eq!(pre_grain, 1, "one whole-object row, as production carries");

    // The repair.
    let (before, live, folded): (i64, i64, i64) =
        sqlx::query_as("SELECT * FROM _facet_regrain_from_events($1)")
            .bind(owner_uuid)
            .fetch_one(&pool)
            .await
            .expect("regrain");
    assert_eq!(before, 1, "it saw the one pre-grain row");
    assert_eq!(live, 2, "and rebuilt it into one row per inner key");
    assert_eq!(
        folded, 0,
        "nothing to fold — a single assert supersedes nothing"
    );

    let after_rebuild: Vec<(serde_json::Value, f64)> = sqlx::query_as(
        "SELECT property_value, weight FROM kb_properties \
          WHERE property_key='facet' AND NOT is_folded AND owner_id=$1 \
          ORDER BY property_value::text",
    )
    .bind(owner_uuid)
    .fetch_all(&pool)
    .await
    .expect("rows after rebuild");
    assert_eq!(
        after_rebuild, after_write,
        "the rebuild must land exactly what the projector wrote — same marks, same weights"
    );

    // Idempotence: the operator primitive is safe to re-run, because it derives from the ledger.
    let (before2, live2, _): (i64, i64, i64) =
        sqlx::query_as("SELECT * FROM _facet_regrain_from_events($1)")
            .bind(owner_uuid)
            .fetch_one(&pool)
            .await
            .expect("second regrain");
    assert_eq!((before2, live2), (2, 2), "re-running changes nothing");
}
