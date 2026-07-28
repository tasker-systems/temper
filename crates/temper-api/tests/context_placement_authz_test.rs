#![cfg(feature = "test-db")]
//! **F-2** — placing a resource into a context requires WRITE authority on that context, not READ.
//!
//! Surfaces resolve a context ref through `resolve_context_ref`, which gates on `context_visible_to`
//! → `context_readable_by_profile`. Read is strictly broader than `context_authorable_by_profile`:
//! it inherits up the team-enclosure chain and admits `watcher` and read-only grants. Before this
//! gate, authority to *see* a context conferred authority to *place content in* it.
//!
//! Placement has two verbs and both are covered here, because they are one decision:
//!
//!   1. **create** into a context (`DbBackend::create_resource`, `HomeAnchor::Context`)
//!   2. **re-home** an existing resource into one (`DbBackend::update_resource`, `move_to.context_to`)
//!
//! The re-home is the sharper half — create places new content, re-home drags *existing* content into
//! a container the actor has no authority over — and `update`'s own `check_can_modify_next` authorizes
//! the resource being moved, never the destination.
//!
//! Every test asserts against a `watcher` on the OWNING team, and each first asserts that the watcher
//! genuinely **can read** the context. Without that, a green could mean "the context was invisible" —
//! the wrong gate — instead of "read was allowed and write was refused", which is the whole finding.
//! Each denial is also checked to have written nothing (auth before writes).

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use temper_services::backend::DbBackend;
use temper_services::services::{context_service, team_service};
use temper_workflow::operations::{Backend, CreateResource, MoveSpec, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Create a root team owned by `owner`. Returns the team id.
async fn create_team(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    team_service::create_team(
        pool,
        ProfileId::from(owner),
        &TeamCreateRequest {
            slug: slug.to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        },
    )
    .await
    .expect("create team")
    .id
}

/// Add `profile` to `team` at `role`, acting as the team owner.
async fn add_member(pool: &PgPool, actor: Uuid, team_id: Uuid, profile: Uuid, role: TeamRole) {
    team_service::add_member(
        pool,
        ProfileId::from(actor),
        team_id,
        &AddMemberRequest {
            profile_id: profile,
            role,
        },
    )
    .await
    .expect("add member");
}

/// A team-owned context plus its owning team: `(team_id, owner_profile, context_id)`.
async fn team_owned_context(pool: &PgPool, slug: &str) -> (Uuid, Uuid, Uuid) {
    let owner_email = format!("{slug}-owner-{}@example.com", Uuid::new_v4());
    let owner = common::fixtures::create_test_profile(pool, &owner_email).await;
    let team_id = create_team(pool, owner, &format!("{slug}-team-{}", Uuid::new_v4())).await;
    let ctx = context_service::create(pool, "kb_teams", team_id, "shared")
        .await
        .expect("create team-owned context");
    (team_id, owner, *ctx.id)
}

/// The two predicates the finding is about, read from the live DB.
async fn read_and_write(pool: &PgPool, profile: Uuid, context: Uuid) -> (bool, bool) {
    let readable: bool = sqlx::query_scalar("SELECT context_visible_to($1, $2)")
        .bind(profile)
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("read predicate");
    let writable: bool = sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(profile)
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("write predicate");
    (readable, writable)
}

/// Resources currently homed in `context` — the delta a denied placement must not move.
///
/// `kb_resource_homes` carries a UNIQUE on `resource_id`, so a re-home rewrites the single home row
/// rather than appending one; counting rows per anchor measures placement for both verbs.
async fn resources_in_context(pool: &PgPool, context: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_homes \
         WHERE anchor_table = 'kb_contexts' AND anchor_id = $1",
    )
    .bind(context)
    .fetch_one(pool)
    .await
    .expect("count homes")
}

fn create_cmd(context: Uuid, slug: &str) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: format!("F-2 placement {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://f2-{slug}-{}", Uuid::new_v4())),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

fn rehome_cmd(resource: Uuid, dest: Uuid) -> UpdateResource {
    UpdateResource {
        open_meta_add: None,
        resource: ResourceId::from(resource),
        title: None,
        slug: None,
        body: None,
        managed_meta: None,
        open_meta: None,
        goal: None,
        move_to: Some(MoveSpec {
            context_to: Some(ContextId::from(dest)),
            type_to: None,
        }),
        context_ref: None,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

// ─── create ──────────────────────────────────────────────────────────────────

/// The finding itself: a `watcher` on the owning team can READ the context and must not be able to
/// CREATE into it. The positive control in the same test (a `member` succeeds) is what keeps this
/// from passing for the wrong reason — a blanket denial would satisfy the first half alone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn watcher_cannot_create_into_team_context(pool: PgPool) {
    let (team_id, team_owner, context) = team_owned_context(&pool, "create").await;

    let watcher_email = format!("f2-watcher-{}@example.com", Uuid::new_v4());
    let watcher = common::fixtures::create_test_profile(&pool, &watcher_email).await;
    add_member(&pool, team_owner, team_id, watcher, TeamRole::Watcher).await;

    // The gap, stated as data: read says yes, write says no.
    let (readable, writable) = read_and_write(&pool, watcher, context).await;
    assert!(
        readable,
        "a watcher must genuinely READ the context — otherwise the denial below is the wrong gate"
    );
    assert!(!writable, "a watcher must NOT be authorable on the context");

    let baseline = resources_in_context(&pool, context).await;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(watcher));
    let denied = backend
        .create_resource(create_cmd(context, "watcher"))
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "a read-only member must be Forbidden from creating into the context, got {denied:?}"
    );
    assert_eq!(
        resources_in_context(&pool, context).await,
        baseline,
        "a denied create must write no home row (auth before writes)"
    );

    // Positive control: an authoring role on the same context still works.
    let member_email = format!("f2-member-{}@example.com", Uuid::new_v4());
    let member = common::fixtures::create_test_profile(&pool, &member_email).await;
    add_member(&pool, team_owner, team_id, member, TeamRole::Member).await;
    DbBackend::new(pool.clone(), ProfileId::from(member))
        .create_resource(create_cmd(context, "member"))
        .await
        .expect("a member with an authoring role must still create into the context");
    assert_eq!(
        resources_in_context(&pool, context).await,
        baseline + 1,
        "the permitted create writes exactly one home row"
    );
}

/// A non-member cannot create into a team context either. Distinct from the watcher case: here the
/// context is not even readable, so this pins that closing the read/write gap did not accidentally
/// make an unreadable context *reachable* through some other arm.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn outsider_cannot_create_into_team_context(pool: PgPool) {
    let (_team_id, _team_owner, context) = team_owned_context(&pool, "outsider").await;

    let email = format!("f2-outsider-{}@example.com", Uuid::new_v4());
    let outsider = common::fixtures::create_test_profile(&pool, &email).await;

    let (readable, writable) = read_and_write(&pool, outsider, context).await;
    assert!(!readable, "an outsider must not read the context");
    assert!(!writable, "an outsider must not be authorable on it");

    let baseline = resources_in_context(&pool, context).await;
    let denied = DbBackend::new(pool.clone(), ProfileId::from(outsider))
        .create_resource(create_cmd(context, "outsider"))
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "an outsider must be Forbidden from creating into the context, got {denied:?}"
    );
    assert_eq!(
        resources_in_context(&pool, context).await,
        baseline,
        "a denied create must write no home row"
    );
}

// ─── re-home ─────────────────────────────────────────────────────────────────

/// The second site, and the sharper one: the caller owns the resource outright (so
/// `check_can_modify_next` passes) and is only a `watcher` on the destination. Without the
/// destination gate, they could drag their content into a container they cannot write.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn watcher_cannot_rehome_into_team_context(pool: PgPool) {
    let (team_id, team_owner, dest) = team_owned_context(&pool, "rehome").await;

    let email = format!("f2-rehome-watcher-{}@example.com", Uuid::new_v4());
    let (mover, own_context) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    add_member(&pool, team_owner, team_id, mover, TeamRole::Watcher).await;

    let (readable, writable) = read_and_write(&pool, mover, dest).await;
    assert!(readable, "the mover must genuinely READ the destination");
    assert!(!writable, "the mover must NOT be authorable on it");

    // A resource the mover owns outright, in their own context.
    let backend = DbBackend::new(pool.clone(), ProfileId::from(mover));
    let created = backend
        .create_resource(create_cmd(own_context, "mine"))
        .await
        .expect("create into own context must succeed");
    let resource = Uuid::from(created.value.id);

    let baseline = resources_in_context(&pool, dest).await;
    let denied = backend.update_resource(rehome_cmd(resource, dest)).await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "re-home into a read-only context must be Forbidden, got {denied:?}"
    );
    assert_eq!(
        resources_in_context(&pool, dest).await,
        baseline,
        "a denied re-home must move nothing (auth before writes)"
    );
    // And the resource is still where it started — the denial is not a half-applied move.
    assert_eq!(
        resources_in_context(&pool, own_context).await,
        1,
        "the resource must remain homed in its original context"
    );
}

/// Positive control for the re-home arm: promote the same actor to an authoring role and the identical
/// move succeeds. Without this, `watcher_cannot_rehome_into_team_context` would also pass if re-home
/// were broken outright.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn member_can_rehome_into_team_context(pool: PgPool) {
    let (team_id, team_owner, dest) = team_owned_context(&pool, "rehome-ok").await;

    let email = format!("f2-rehome-member-{}@example.com", Uuid::new_v4());
    let (mover, own_context) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    add_member(&pool, team_owner, team_id, mover, TeamRole::Member).await;

    let (_readable, writable) = read_and_write(&pool, mover, dest).await;
    assert!(writable, "a member must be authorable on the team context");

    let backend = DbBackend::new(pool.clone(), ProfileId::from(mover));
    let created = backend
        .create_resource(create_cmd(own_context, "movable"))
        .await
        .expect("create into own context must succeed");
    let resource = Uuid::from(created.value.id);

    let baseline = resources_in_context(&pool, dest).await;
    backend
        .update_resource(rehome_cmd(resource, dest))
        .await
        .expect("a member with an authoring role must be able to re-home into the context");
    assert_eq!(
        resources_in_context(&pool, dest).await,
        baseline + 1,
        "the permitted re-home lands exactly one resource in the destination"
    );
}
