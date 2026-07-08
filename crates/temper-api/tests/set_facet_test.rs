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
            resource,
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

    // Owner: succeeds, returns the property id.
    let CommandOutput {
        value: property_id, ..
    } = owner_backend
        .set_facet(SetFacet {
            resource,
            values: serde_json::json!({"k": "v"}),
            weight: 1.0,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("owner facet set must succeed");

    assert_ne!(
        property_id,
        PropertyId::from(Uuid::nil()),
        "set_facet must return a real property id"
    );

    let (stored_key, stored_value): (String, serde_json::Value) =
        sqlx::query_as("SELECT property_key, property_value FROM kb_properties WHERE id = $1")
            .bind(property_id.uuid())
            .fetch_one(&pool)
            .await
            .expect("the facet property row must exist");
    assert_eq!(stored_key, "facet");
    assert_eq!(stored_value, serde_json::json!({"k": "v"}));
}
