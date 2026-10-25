//! `filtered_visible_page`'s `ids` filter — the bulk read a rendered body resolves its links
//! through, one request for every resource the body names.
//!
//! The contract is the intersection and nothing else: the rows are the visible resources among the
//! ids. An id that does not exist, was deleted, or is not visible to the caller is simply absent
//! from the rows, and the response carries no other trace of it — no per-id status, no error, no
//! count. So the no-oracle arm compares the WHOLE serialized response, not only `rows` and `total`:
//! a facet histogram or a `truncated` flag that moved would say the same thing a row would.
//!
//! An unfinished upload (`ingest_state` other than `complete`) stays out, exactly as it does
//! without `ids` — reading by id is still a list read, and a list read returns documents.

#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::query::id_set::MAX_ID_SET_IDS;
use temper_services::backend::{substrate_read, DbBackend};
use temper_services::error::ApiError;
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::{ResourceListParams, ResourceListResponse};

/// Seed a substrate profile + a profile-owned `temper` context. Mirrors the inlined fixture in
/// `has_artifacts_filter_test.rs`.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// Create one resource through the real write path, and return its id.
async fn mk(backend: &DbBackend, context: uuid::Uuid, slug: &str, doctype: &str) -> uuid::Uuid {
    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: slug.to_string(),
            doctype: doctype.to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some(format!("test://{slug}")),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create");
    created.value.id.into()
}

fn csv(ids: &[uuid::Uuid]) -> String {
    ids.iter()
        .map(uuid::Uuid::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

async fn list(
    pool: &PgPool,
    principal: uuid::Uuid,
    params: ResourceListParams,
) -> Result<ResourceListResponse, ApiError> {
    substrate_read::list_select(pool, ProfileId::from(principal), params).await
}

async fn list_ids(pool: &PgPool, principal: uuid::Uuid, ids: &[uuid::Uuid]) -> Vec<uuid::Uuid> {
    list(
        pool,
        principal,
        ResourceListParams {
            ids: Some(csv(ids)),
            ..Default::default()
        },
    )
    .await
    .expect("list")
    .rows
    .into_iter()
    .map(|r| r.id.into())
    .collect()
}

fn bad_request(result: Result<ResourceListResponse, ApiError>) -> String {
    match result {
        Err(ApiError::BadRequest(msg)) => msg,
        Err(other) => panic!("expected BadRequest, got {other:?}"),
        Ok(_) => panic!("expected BadRequest, got a page"),
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn absent_ids_param_changes_nothing(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "ids-absent@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    mk(&backend, context, "one", "note").await;
    mk(&backend, context, "two", "note").await;

    let scoped = |ids: Option<String>| ResourceListParams {
        context_ref: Some(context.to_string()),
        ids,
        ..Default::default()
    };
    let without = list(&pool, profile, scoped(None)).await.expect("list");
    // A CSV that trims away entirely is no filter, as `cogmap_ids` treats it.
    let empty = list(&pool, profile, scoped(Some(" , ".into())))
        .await
        .expect("list");
    assert_eq!(without.rows.len(), 2);
    assert_eq!(
        serde_json::to_value(&without).unwrap(),
        serde_json::to_value(&empty).unwrap()
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ids_returns_only_the_named_visible_resources(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "ids-named@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let a = mk(&backend, context, "a", "note").await;
    let b = mk(&backend, context, "b", "note").await;
    mk(&backend, context, "c", "note").await;

    let mut got = list_ids(&pool, profile, &[a, b]).await;
    got.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(got, want);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn invisible_and_nonexistent_ids_are_indistinguishable_from_unasked(pool: PgPool) {
    let (a_profile, a_context) = seed_profile_with_context(&pool, "ids-a@example.com").await;
    let (b_profile, b_context) = seed_profile_with_context(&pool, "ids-b@example.com").await;
    let a_backend = DbBackend::new(pool.clone(), ProfileId::from(a_profile));
    let b_backend = DbBackend::new(pool.clone(), ProfileId::from(b_profile));
    let a_visible = mk(&a_backend, a_context, "mine", "note").await;
    let b_private = mk(&b_backend, b_context, "theirs", "note").await;
    let deleted = mk(&a_backend, a_context, "gone", "note").await;
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(deleted)
        .execute(&pool)
        .await
        .expect("deactivate");

    let ask = |ids: &[uuid::Uuid]| ResourceListParams {
        ids: Some(csv(ids)),
        ..Default::default()
    };
    let mixed = list(
        &pool,
        a_profile,
        ask(&[a_visible, b_private, uuid::Uuid::now_v7(), deleted]),
    )
    .await
    .expect("list");
    let alone = list(&pool, a_profile, ask(&[a_visible]))
        .await
        .expect("list");

    assert_eq!(mixed.rows.len(), 1, "only the visible resource comes back");
    assert_eq!(
        serde_json::to_value(&mixed).unwrap(),
        serde_json::to_value(&alone).unwrap(),
        "the response must carry no trace of the ids it did not return"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ids_compose_with_other_filters_as_and(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "ids-and@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let note = mk(&backend, context, "a-note", "note").await;
    let task = mk(&backend, context, "a-task", "task").await;

    let got: Vec<uuid::Uuid> = list(
        &pool,
        profile,
        ResourceListParams {
            ids: Some(csv(&[note, task])),
            doc_type_name: Some("note".into()),
            ..Default::default()
        },
    )
    .await
    .expect("list")
    .rows
    .into_iter()
    .map(|r| r.id.into())
    .collect();
    assert_eq!(got, vec![note]);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ids_never_return_an_interrupted_ingest(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "ids-ingest@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let done = mk(&backend, context, "done", "note").await;
    let partial = mk(&backend, context, "partial", "note").await;
    sqlx::query("UPDATE kb_resources SET ingest_state = 'in_progress' WHERE id = $1")
        .bind(partial)
        .execute(&pool)
        .await
        .expect("interrupt");

    assert_eq!(list_ids(&pool, profile, &[done, partial]).await, vec![done]);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ids_rows_are_the_list_row_shape(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "ids-shape@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let id = mk(&backend, context, "shaped", "note").await;

    let by_id = list(
        &pool,
        profile,
        ResourceListParams {
            ids: Some(id.to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("list");
    let by_context = list(
        &pool,
        profile,
        ResourceListParams {
            context_ref: Some(context.to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("list");

    let row = serde_json::to_value(&by_id.rows[0]).unwrap();
    assert_eq!(row, serde_json::to_value(&by_context.rows[0]).unwrap());
    assert!(row.get("content").is_none(), "a list row carries no body");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn malformed_id_is_400(pool: PgPool) {
    let (profile, _) = seed_profile_with_context(&pool, "ids-malformed@example.com").await;
    let msg = bad_request(
        list(
            &pool,
            profile,
            ResourceListParams {
                ids: Some("not-a-uuid".into()),
                ..Default::default()
            },
        )
        .await,
    );
    assert!(
        msg.contains("not-a-uuid"),
        "names the malformed input: {msg}"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ids_at_cap_passes_and_over_cap_is_400_naming_the_field_without_values(pool: PgPool) {
    let (profile, _) = seed_profile_with_context(&pool, "ids-cap@example.com").await;
    let nils = |n: usize| ResourceListParams {
        ids: Some(vec![uuid::Uuid::nil().to_string(); n].join(",")),
        ..Default::default()
    };

    assert!(list(&pool, profile, nils(MAX_ID_SET_IDS)).await.is_ok());

    let msg = bad_request(list(&pool, profile, nils(MAX_ID_SET_IDS + 1)).await);
    assert!(msg.contains("`ids`"), "names the field: {msg}");
    assert!(!msg.contains("00000000-"), "never echoes the values: {msg}");
}
