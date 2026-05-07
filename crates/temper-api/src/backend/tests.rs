//! Trait-impl integration tests for `DbBackend`.
//!
//! Each test uses `#[sqlx::test(migrator = "crate::MIGRATOR")]` for an
//! isolated per-test database.

#![cfg(test)]

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::operations::{Backend, CreateResource, DomainEvent, Surface};
use temper_core::types::ids::ProfileId;
use temper_core::types::managed_meta::ManagedMeta;

use crate::backend::DbBackend;

// Well-known UUIDs from the R2 seed migration. Mirrors the constants in
// `crates/temper-api/tests/common/fixtures.rs`; copied here because src/
// can't depend on the integration-test crate's helpers.
const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
const TEMPER_CONTEXT_NAME: &str = "temper";

fn system_profile() -> ProfileId {
    ProfileId(Uuid::parse_str(SYSTEM_PROFILE_ID).unwrap())
}

fn make_backend(pool: PgPool) -> DbBackend {
    DbBackend::new(pool, system_profile(), "test".to_string(), Surface::ApiHttp)
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn create_resource_inserts_row_and_emits_event(pool: PgPool) {
    let backend = make_backend(pool);
    // body: None → content is empty string, which bypasses the chunks_packed
    // requirement in the no-ingest-pipeline path (ingest_service only requires
    // chunks_packed when content is non-empty and the pipeline isn't compiled in).
    let cmd = CreateResource {
        slug: "create-test-1".to_string(),
        doctype: "task".to_string(),
        context: TEMPER_CONTEXT_NAME.to_string(),
        title: "Create test 1".to_string(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin: Surface::ApiHttp,
    };

    let out = backend.create_resource(cmd).await.expect("create succeeds");

    assert_eq!(out.value.slug.as_deref(), Some("create-test-1"));
    assert_eq!(out.value.title, "Create test 1");
    assert_eq!(out.events.len(), 1);
    match &out.events[0] {
        DomainEvent::DbResourceCreated { resource_id } => {
            assert_eq!(*resource_id, out.value.id);
        }
        other => panic!("expected DbResourceCreated, got {other:?}"),
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn create_resource_unknown_doctype_returns_temper_error(pool: PgPool) {
    let backend = make_backend(pool);
    let cmd = CreateResource {
        slug: "create-test-bad".to_string(),
        doctype: "widget".to_string(),
        context: TEMPER_CONTEXT_NAME.to_string(),
        title: "Bad doctype".to_string(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin: Surface::ApiHttp,
    };

    let err = backend.create_resource(cmd).await.unwrap_err();
    // Whatever specific TemperError variant ingest_service returns for an
    // unknown doctype (likely BadRequest after the From<ApiError> conversion).
    // Asserting it's an error of any non-Internal kind is the contract.
    use temper_core::error::TemperError;
    assert!(
        !matches!(err, TemperError::Api(_)),
        "expected typed variant for unknown doctype, got generic Api: {err:?}"
    );
}
