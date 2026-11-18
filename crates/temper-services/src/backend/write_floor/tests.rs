//! Witnesses for both floors. Every state is made by a real path: the resource by
//! `writes::create_resource_with`, the tombstone by `SeedAction::ResourceDelete`, the husk by
//! `execute_resource_erasure` under a minted `SystemAdmin` proof (the pattern in
//! `resource_erasure_service`'s tests). Only the read grant is a fixture row, as in
//! `temper-substrate/tests/resource_husk_held_by.rs`.
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::EntityId;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::ContextId;
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{self, CreateParams};
use temper_workflow::operations::Surface;

use super::*;
use crate::auth::SystemAdmin;
use crate::services::resource_erasure_service::{
    execute_resource_erasure, ResourceErasureOutcome, ResourceErasureRequest,
};
use crate::test_support::{self, context, entity, profile};

/// A principal: profile, its `<handle>@web` emitter entity, and a personal context.
struct Principal {
    profile: ProfileId,
    emitter: EntityId,
    home: ContextId,
}

/// The handle is the FULL id: two UUIDv7s minted in one millisecond share leading bytes, so a
/// truncated handle collides on `kb_profiles_handle_key`.
async fn principal(pool: &PgPool) -> Principal {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    profile::insert_with_id(pool, id, &handle).await;
    let emitter = entity::insert(pool, id, &format!("{handle}@web")).await;
    let home = context::insert_personal(pool, id, "home", "Home").await;
    Principal {
        profile: ProfileId::from(id),
        emitter: EntityId::from(emitter),
        home: ContextId::from(home),
    }
}

/// An operator's sealed proof, minted through the real gate after `grant_governance`.
async fn operator(pool: &PgPool) -> SystemAdmin {
    let op = principal(pool).await;
    test_support::grant_governance(pool, op.profile.uuid()).await;
    test_support::system_admin_proof_for(pool, op.profile.uuid()).await
}

/// A live resource created through the REAL create path, homed in `owner`'s context.
async fn resource(pool: &PgPool, owner: &Principal) -> ResourceId {
    let origin = format!("test://write-floor-{}", Uuid::now_v7());
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "write floor subject",
            origin_uri: &origin,
            body: "body under the write floor",
            doc_type: "research",
            home: AnchorRef::context(owner.home),
            owner: owner.profile,
            originator: owner.profile,
            emitter: owner.emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create resource through the real path")
}

/// Soft-delete `resource` through the real path: a tombstone, never a husk.
async fn tombstone(pool: &PgPool, owner: &Principal, resource: ResourceId) {
    let mut tx = pool.begin().await.expect("begin");
    fire(
        &mut tx,
        SeedAction::ResourceDelete {
            resource,
            emitter: owner.emitter,
        },
    )
    .await
    .expect("soft delete through the real path");
    tx.commit().await.expect("commit");
    let (is_active, erased): (bool, bool) =
        sqlx::query_as("SELECT is_active, erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
            .bind(resource.uuid())
            .fetch_one(pool)
            .await
            .expect("row");
    assert!(
        !is_active && !erased,
        "precondition: a tombstone, not a husk"
    );
}

/// Erase `resource` through the real act; asserts it completed and left a husk.
async fn erase(pool: &PgPool, resource: ResourceId) {
    let op = operator(pool).await;
    let outcome = execute_resource_erasure(
        pool,
        None,
        &op,
        ResourceErasureRequest {
            resource,
            also_strike_blobs: &[],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("the act answers");
    assert!(
        matches!(outcome, ResourceErasureOutcome::Completed(_)),
        "the act must complete, got {outcome:?}"
    );
    let erased: bool =
        sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
            .bind(resource.uuid())
            .fetch_one(pool)
            .await
            .expect("row");
    assert!(erased, "precondition: the act made a husk");
}

/// A direct profile `can_read` grant, and nothing more.
async fn grant_read(pool: &PgPool, resource: ResourceId, grantee: ProfileId, by: ProfileId) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(resource.uuid())
    .bind(grantee.uuid())
    .bind(by.uuid())
    .execute(pool)
    .await
    .expect("insert read grant");
}

/// The modify floor in a fresh transaction, rolled back after.
async fn modify(
    pool: &PgPool,
    profile: ProfileId,
    resource: ResourceId,
) -> Result<(), TemperError> {
    let mut tx = pool.begin().await.expect("begin");
    let answer = modify_floor_in_tx(&mut tx, profile, resource).await;
    tx.rollback().await.expect("rollback");
    answer
}

/// The liveness floor in a fresh transaction, rolled back after.
async fn liveness(
    pool: &PgPool,
    profile: ProfileId,
    resource: ResourceId,
) -> Result<(), TemperError> {
    let mut tx = pool.begin().await.expect("begin");
    let answer = liveness_floor_in_tx(&mut tx, profile, resource).await;
    tx.rollback().await.expect("rollback");
    answer
}

// ── modify_floor_in_tx ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn modify_floor_admits_the_owner_of_a_live_resource(pool: PgPool) {
    let owner = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    let answer = modify(&pool, owner.profile, r).await;
    assert!(answer.is_ok(), "the owner may modify: {answer:?}");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn modify_floor_forbids_a_non_modifier_of_a_live_resource(pool: PgPool) {
    let owner = principal(&pool).await;
    let stranger = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    let answer = modify(&pool, stranger.profile, r).await;
    assert!(
        matches!(answer, Err(TemperError::Forbidden)),
        "a stranger is forbidden: {answer:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn modify_floor_forbids_the_owner_of_a_tombstone_never_erased(pool: PgPool) {
    let owner = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    tombstone(&pool, &owner, r).await;
    let answer = modify(&pool, owner.profile, r).await;
    assert!(
        matches!(answer, Err(TemperError::Forbidden)),
        "a tombstone is Forbidden, never ResourceErased: {answer:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn modify_floor_answers_erased_to_the_owner_of_a_husk(pool: PgPool) {
    let owner = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    erase(&pool, r).await;
    let answer = modify(&pool, owner.profile, r).await;
    assert!(
        matches!(answer, Err(TemperError::ResourceErased(id)) if id == r),
        "the owner of a husk is told it was erased: {answer:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn modify_floor_answers_erased_to_a_read_grant_holder_of_a_husk(pool: PgPool) {
    let owner = principal(&pool).await;
    let reader = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    grant_read(&pool, r, reader.profile, owner.profile).await;
    assert!(
        matches!(
            modify(&pool, reader.profile, r).await,
            Err(TemperError::Forbidden)
        ),
        "precondition: a read-only grant does not admit a modify while live"
    );
    erase(&pool, r).await;
    let answer = modify(&pool, reader.profile, r).await;
    assert!(
        matches!(answer, Err(TemperError::ResourceErased(id)) if id == r),
        "a read-grant holder of a husk is told it was erased (the read population): {answer:?}"
    );
}

/// Not one of the plan's six: the oracle guard. A husk the caller holds no standing on answers
/// exactly as an unknown id does.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn modify_floor_forbids_a_stranger_to_a_husk(pool: PgPool) {
    let owner = principal(&pool).await;
    let stranger = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    erase(&pool, r).await;
    let answer = modify(&pool, stranger.profile, r).await;
    assert!(
        matches!(answer, Err(TemperError::Forbidden)),
        "a stranger to a husk gets Forbidden, never ResourceErased: {answer:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn modify_floor_forbids_an_unknown_id(pool: PgPool) {
    let caller = principal(&pool).await;
    let answer = modify(&pool, caller.profile, ResourceId::from(Uuid::now_v7())).await;
    assert!(
        matches!(answer, Err(TemperError::Forbidden)),
        "an unknown id is Forbidden, not a distinct answer: {answer:?}"
    );
}

// ── liveness_floor_in_tx ────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn liveness_floor_admits_a_non_owner_of_a_live_resource(pool: PgPool) {
    let owner = principal(&pool).await;
    let stranger = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    let answer = liveness(&pool, stranger.profile, r).await;
    assert!(
        answer.is_ok(),
        "liveness checks no authority, so a non-owner passes on a live resource: {answer:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn liveness_floor_forbids_a_tombstone(pool: PgPool) {
    let owner = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    tombstone(&pool, &owner, r).await;
    let answer = liveness(&pool, owner.profile, r).await;
    assert!(
        matches!(answer, Err(TemperError::Forbidden)),
        "a tombstone fails liveness as Forbidden, never ResourceErased: {answer:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn liveness_floor_answers_erased_to_the_holder_of_a_husk(pool: PgPool) {
    let owner = principal(&pool).await;
    let r = resource(&pool, &owner).await;
    erase(&pool, r).await;
    let answer = liveness(&pool, owner.profile, r).await;
    assert!(
        matches!(answer, Err(TemperError::ResourceErased(id)) if id == r),
        "the holder of a husk is told it was erased: {answer:?}"
    );
}
