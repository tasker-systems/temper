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
