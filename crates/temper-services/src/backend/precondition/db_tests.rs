//! The judgment's own read gate, asked directly: a race between the locked gate and the
//! comparison is not reproducible on demand, so the branch it would take is pinned here.
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_workflow::operations::{CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

use super::*;
use crate::backend::DbBackend;
use crate::test_support::{context, entity, profile};

async fn profile_with_context(pool: &PgPool) -> (Uuid, Uuid) {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    profile::insert_with_id(pool, id, &handle).await;
    entity::insert(pool, id, &format!("{handle}@web")).await;
    (id, context::insert_personal(pool, id, "home", "Home").await)
}

/// FAILS IF the comparison statement stops asking the read predicate: a caller who cannot read
/// the resource would then get the current values.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_caller_who_cannot_read_is_answered_not_visible_and_no_value(pool: PgPool) {
    let (owner, home) = profile_with_context(&pool).await;
    let (stranger, _) = profile_with_context(&pool).await;
    let resource: ResourceId = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "judged".into(),
            doctype: "research".into(),
            home: HomeAnchor::Context(ContextId::from(home)),
            title: "judged".into(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: Some(json!({"claim": "held"})),
            goal: None,
            origin_uri: Some(format!("test://judged/{}", Uuid::now_v7())),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create")
        .value
        .id;
    let guard: MetaExpect =
        serde_json::from_value(json!({"open": {"claim": "other"}})).expect("parses");
    let mut conn = pool.acquire().await.expect("connection");

    match judge(&mut conn, ProfileId::from(owner), resource, &guard)
        .await
        .expect("judge")
    {
        Judgment::Fails(f) => assert_eq!(f.failed[0].current, json!("held")),
        other => panic!("the owner reads the value: {other:?}"),
    }
    match judge(&mut conn, ProfileId::from(stranger), resource, &guard)
        .await
        .expect("judge")
    {
        Judgment::NotVisible => {}
        other => panic!("a non-reader is answered NotVisible, never a value: {other:?}"),
    }
}
