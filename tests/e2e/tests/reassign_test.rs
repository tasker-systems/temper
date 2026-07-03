#![cfg(feature = "test-db")]

//! Drive `temper resource reassign` end-to-end (CLI → API → DB) and assert the
//! owner change lands in `kb_resource_homes` and flips `resources_visible_to`.
//! The service/auth matrix is covered by unit tests in `reassign_service`; this
//! proves the CLI → client → API → substrate wire end-to-end.

mod common;

use temper_workflow::types::resource::ResourceCreateRequest;
use uuid::Uuid;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_reassign_moves_ownership(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // The owner (e2e-test-user, carried by app.token) creates a context + resource.
    let context = app
        .client
        .contexts()
        .create("e2e-reassign", None)
        .await
        .expect("context create failed");
    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/reassign".to_string(),
            title: "Reassign E2E".to_string(),
            slug: None,
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    // Seed a recipient profile (bob) directly — the owner path lets an owner
    // reassign to any valid profile.
    let bob: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ('bob-reassign-e2e', 'bob-reassign-e2e') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("seed bob");

    // Drive the CLI as the owner: temper resource reassign <uuid> --to <bob>.
    let resource_id = Uuid::from(created.id).to_string();
    let bob_str = bob.to_string();
    let out = common::run_temper_cli(
        &app,
        &["resource", "reassign", &resource_id, "--to", &bob_str],
    )
    .await
    .expect("spawn temper cli");
    assert!(
        out.status.success(),
        "reassign CLI failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // The owner moved to bob in the DB.
    let owner: Uuid =
        sqlx::query_scalar("SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id = $1")
            .bind(Uuid::from(created.id))
            .fetch_one(&pool)
            .await
            .expect("owner query");
    assert_eq!(owner, bob, "owner_profile_id should now be bob");

    // And bob (the new owner) sees it via the visibility function.
    let visible: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(bob)
    .bind(Uuid::from(created.id))
    .fetch_one(&pool)
    .await
    .expect("visibility query");
    assert!(visible, "bob (new owner) should see the resource");
}
