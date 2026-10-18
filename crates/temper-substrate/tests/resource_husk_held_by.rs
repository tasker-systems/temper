#![cfg(feature = "test-db")]
//! `resource_husk_held_by` (migration 20260930000060; resource erasure spec D6): who may be told a
//! resource was erased. The husk is made by the REAL act (`resource_erasure_execute`), the
//! tombstone by the REAL soft-delete path (`SeedAction::ResourceDelete`), never by a hand-written
//! `UPDATE`. Placement follows `resource_erasure_act.rs`: a `test-db` file under
//! `crates/temper-substrate/tests/` on `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`.
//!
//! `resource_erased` is seeded by `fixtures/seeds/system.yaml`, so no re-registration is needed
//! after `reset_schema` (unlike `block_provenance_annotated`).

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

const PROSE: &str = "prose the act will erase";

struct World {
    resource: ResourceId,
    owner: Uuid,
    direct_grantee: Uuid,
    team_grantee: Uuid,
    /// Reads the resource ONLY through the context it is homed in (team-owned context, no grant).
    context_member: Uuid,
    stranger: Uuid,
    operator_entity: Uuid,
}

async fn held(pool: &PgPool, profile: Uuid, resource: ResourceId) -> bool {
    sqlx::query_scalar("SELECT resource_husk_held_by($1, $2)")
        .bind(profile)
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn visible(pool: &PgPool, profile: Uuid, resource: ResourceId) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) WHERE resource_id = $2)",
    )
    .bind(profile)
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn grant_read(pool: &PgPool, resource: ResourceId, table: &str, principal: Uuid, by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, $2, $3, true, $4)",
    )
    .bind(resource.uuid())
    .bind(table)
    .bind(principal)
    .bind(by)
    .execute(pool)
    .await
    .unwrap();
}

/// One live resource, homed in a team-owned context, owned by `owner`, with every population the
/// predicate distinguishes built around it. The act has NOT run.
async fn world(pool: &PgPool) -> World {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
    let system: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let operator_entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(system)
            .fetch_one(pool)
            .await
            .unwrap();

    let owner = common::create_profile(pool, "husk-owner@example.test").await;
    let direct_grantee = common::create_profile(pool, "husk-direct@example.test").await;
    let team_grantee = common::create_profile(pool, "husk-teamgrant@example.test").await;
    let context_member = common::create_profile(pool, "husk-context@example.test").await;
    let stranger = common::create_profile(pool, "husk-stranger@example.test").await;

    let home_team = common::create_team(pool, "husk-home-team").await;
    common::add_team_member(pool, home_team, context_member).await;
    let grant_team = common::create_team(pool, "husk-grant-team").await;
    common::add_team_member(pool, grant_team, team_grantee).await;

    let home = ContextId::from(
        common::insert_context(pool, "kb_teams", home_team, "husk-home", "husk-home")
            .await
            .unwrap(),
    );
    let chunk = IncomingChunk {
        chunk_index: 0,
        content_hash: hex::encode(sha2::Sha256::digest(PROSE.trim())),
        content: PROSE.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: String::new(),
        heading_depth: 0,
    };
    let resource = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "husk subject",
            origin_uri: "test://husk-held-by",
            body: PROSE,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(operator_entity),
            properties: &[],
            chunks: Some(vec![chunk]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create the resource through the real path");

    grant_read(pool, resource, "kb_profiles", direct_grantee, owner).await;
    grant_read(pool, resource, "kb_teams", grant_team, owner).await;

    World {
        resource,
        owner,
        direct_grantee,
        team_grantee,
        context_member,
        stranger,
        operator_entity,
    }
}

/// The real act, executed as the boot-seeded system operator.
async fn erase(pool: &PgPool, w: &World) {
    sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(w.resource.uuid())
        .bind(w.operator_entity)
        .bind(w.operator_entity)
        .bind(Uuid::now_v7())
        .execute(pool)
        .await
        .expect("the act completes");
}

/// Owner, direct-grant and team-grant holders each get true on a husk. One assertion per arm, so
/// dropping any one arm from the function fails exactly its witness. The preconditions pin that
/// the same callers read it as live BEFORE the act (so the fixture reaches R by each arm) and that
/// the husk is no longer visible to them (`resources_visible_to` keeps its `is_active` floor).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_owner_and_both_grant_holders_hold_a_husk(pool: PgPool) {
    let w = world(&pool).await;
    for (who, arm) in [
        (w.owner, "owner"),
        (w.direct_grantee, "direct grant"),
        (w.team_grantee, "team grant"),
    ] {
        assert!(
            visible(&pool, who, w.resource).await,
            "{arm}: reaches R while live"
        );
        assert!(
            !held(&pool, who, w.resource).await,
            "{arm}: a live resource is not a husk, so no one holds one"
        );
    }

    erase(&pool, &w).await;

    let erased_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT erased_at FROM kb_resources WHERE id = $1")
            .bind(w.resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(erased_at.is_some(), "precondition: the act made a husk");
    assert!(
        held(&pool, w.owner, w.resource).await,
        "the owner of a husk holds it"
    );
    assert!(
        held(&pool, w.direct_grantee, w.resource).await,
        "a direct can_read grantee of a husk holds it"
    );
    assert!(
        held(&pool, w.team_grantee, w.resource).await,
        "a team can_read grantee (through profile_reachable_teams) of a husk holds it"
    );
    assert!(
        !visible(&pool, w.owner, w.resource).await,
        "precondition: resources_visible_to still hides the husk, so only this predicate can say 410"
    );
}

/// A member of the context R was homed in, with no grant, reads R while it is live and gets false
/// on the husk; a stranger gets false. The context arm is excluded on purpose (D6).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_context_member_and_a_stranger_do_not_hold_a_husk(pool: PgPool) {
    let w = world(&pool).await;
    assert!(
        visible(&pool, w.context_member, w.resource).await,
        "precondition: the context member reaches R through the context arm while it is live, or \
         the false below would be vacuous"
    );
    assert!(
        !visible(&pool, w.stranger, w.resource).await,
        "precondition: the stranger reaches R by no arm"
    );

    erase(&pool, &w).await;

    assert!(
        !held(&pool, w.context_member, w.resource).await,
        "context-homed standing is not standing on the husk"
    );
    assert!(
        !held(&pool, w.stranger, w.resource).await,
        "a stranger never holds a husk"
    );
}

/// The owner of a tombstone (soft-deleted through the real path, `erased_at` NULL) gets false: a
/// tombstone is never mistaken for an erasure.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_owner_of_a_tombstone_does_not_hold_a_husk(pool: PgPool) {
    let w = world(&pool).await;

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceDelete {
            resource: w.resource,
            emitter: EntityId::from(w.operator_entity),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (is_active, erased_at): (bool, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT is_active, erased_at FROM kb_resources WHERE id = $1")
            .bind(w.resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !is_active && erased_at.is_none(),
        "precondition: a tombstone, not a husk"
    );
    let owner_row: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_resource_homes WHERE resource_id = $1 AND owner_profile_id = $2)",
    )
    .bind(w.resource.uuid())
    .bind(w.owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        owner_row,
        "precondition: the owner arm reaches the tombstone (only erased_at excludes it)"
    );

    assert!(
        !held(&pool, w.owner, w.resource).await,
        "the owner of a tombstone does not hold a husk: the 410 is for erasure, never soft delete"
    );
}
